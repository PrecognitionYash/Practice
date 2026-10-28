// ──────────────────────────────────────────────────────────────────────
// harness-browser/src/driver.rs — Direct CDP Driver & Hardware Event Dispatch
//
// Direct WebSocket protocol driver wrapping `chromiumoxide`.
// Implements:
//  • Stealth launch arguments & Anti-Detection JavaScript injection
//  • Synthetic hardware input dispatch (Bézier curves for mouse move,
//    human micro-tremor jitter, randomized dwell times for clicks)
//  • Keyboard typing dispatch with randomized keystroke inter-arrival
//  • Headless/Headed lifecycle switching for HITL handoff
//  • Cookie export/import for session migration
// ──────────────────────────────────────────────────────────────────────

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    MouseButton,
};
use chromiumoxide::cdp::browser_protocol::network::{
    GetCookiesParams, SetCookieParams, TimeSinceEpoch,
};
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use harness_core::error::{HarnessError, HarnessResult};
use harness_core::schema::CookieEntry;
use rand::Rng;
use tokio::sync::Mutex;
use tracing::{debug, error, info, instrument, warn};

/// Stealth profile configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StealthMode {
    /// Fast mode: Standard anti-automation flags.
    Fast,
    /// Ultra mode: Deep fingerprint masking (WebGL, audio context, fake plugins).
    Ultra,
}

/// Configuration options for the browser driver.
#[derive(Debug, Clone)]
pub struct BrowserDriverConfig {
    /// Whether to run in headless mode.
    pub headless: bool,
    /// Anti-detection stealth configuration.
    pub stealth: StealthMode,
    /// Window viewport dimensions (width, height). Default: (1920, 1080).
    pub window_size: (u32, u32),
    /// Custom user agent string.
    pub user_agent: Option<String>,
    /// Optional path to custom Chrome/Chromium binary.
    pub executable_path: Option<PathBuf>,
}

impl Default for BrowserDriverConfig {
    fn default() -> Self {
        Self {
            headless: true,
            stealth: StealthMode::Ultra,
            window_size: (1920, 1080),
            user_agent: Some(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string(),
            ),
            executable_path: None,
        }
    }
}

/// Point in 2D space for cursor calculations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Direct CDP browser driver with synthetic hardware event dispatch.
pub struct BrowserDriver {
    config: BrowserDriverConfig,
    browser: Arc<Mutex<Browser>>,
    active_page: Arc<Mutex<Page>>,
    current_mouse_pos: Arc<Mutex<Point>>,
    is_headless: Arc<AtomicBool>,
}

impl BrowserDriver {
    /// Launch a new Chromium instance with stealth args and direct CDP connection.
    #[instrument(skip_all)]
    pub async fn launch(config: BrowserDriverConfig) -> HarnessResult<Self> {
        let (browser, page) = Self::spawn_chromium(&config).await?;

        let is_headless = Arc::new(AtomicBool::new(config.headless));
        let driver = Self {
            config,
            browser: Arc::new(Mutex::new(browser)),
            active_page: Arc::new(Mutex::new(page)),
            current_mouse_pos: Arc::new(Mutex::new(Point::new(100.0, 100.0))),
            is_headless,
        };

        // Apply anti-detection evasions to the initial page.
        driver.apply_stealth_scripts().await?;

        info!("browser driver launched successfully");
        Ok(driver)
    }

    /// Internal helper to configure and spawn the Chromium process.
    async fn spawn_chromium(config: &BrowserDriverConfig) -> HarnessResult<(Browser, Page)> {
        let (w, h) = config.window_size;

        let mut builder = BrowserConfig::builder();

        if !config.headless {
            builder = builder.with_head();
        }

        if let Some(ref path) = config.executable_path {
            builder = builder.chrome_executable(path);
        } else if let Some(resolved) = Self::resolve_chrome_executable() {
            builder = builder.chrome_executable(resolved);
        }

        // Create an isolated profile directory in temp to avoid colliding with default user Chrome profile
        let session_data_dir = std::env::temp_dir().join(format!(
            "harness_browser_profile_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&session_data_dir);
        builder = builder.user_data_dir(&session_data_dir);

        // Apply anti-automation, isolation and anti-crashpad flags
        builder = builder.window_size(w, h).args([
            "--disable-blink-features=AutomationControlled",
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--disable-infobars",
            "--disable-dev-shm-usage",
            "--disable-gpu",
            "--disable-features=IsolateOrigins,site-per-process",
            "--disable-extensions",
            "--disable-component-extensions-with-background-pages",
            "--disable-default-apps",
            "--disable-sync",
            "--disable-background-networking",
            "--no-default-browser-check",
            "--no-first-run",
            "--disable-crash-reporter",
            "--no-crashpad",
            "--password-store=basic",
            "--use-mock-keychain",
            "--hide-scrollbars",
            "--mute-audio",
        ]);

        let browser_config = builder.build().map_err(|e| HarnessError::CdpConnection {
            reason: format!("failed to build browser config: {e}"),
        })?;

        let (browser, mut handler) =
            Browser::launch(browser_config)
                .await
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to launch Chromium: {e}"),
                })?;

        // Background handler task to process CDP WebSocket protocol messages.
        tokio::spawn(async move {
            while let Some(res) = handler.next().await {
                if let Err(err) = res {
                    error!("CDP handler event error: {err}");
                    break;
                }
            }
            debug!("CDP handler loop terminated");
        });

        let page =
            browser
                .new_page("about:blank")
                .await
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to create initial page: {e}"),
                })?;

        Ok((browser, page))
    }

    /// Discovers system-installed Chrome/Chromium binaries across OS distributions.
    pub fn resolve_chrome_executable() -> Option<PathBuf> {
        if let Ok(p) =
            std::env::var("CHROME_PATH").or_else(|_| std::env::var("HARNESS_CHROME_PATH"))
        {
            let path = PathBuf::from(p);
            if path.exists() {
                return Some(path);
            }
        }

        #[cfg(target_os = "macos")]
        {
            let candidates = [
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ];
            for c in candidates {
                let p = PathBuf::from(c);
                if p.exists() {
                    return Some(p);
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            let candidates = [
                "/usr/bin/google-chrome-stable",
                "/usr/bin/google-chrome",
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
                "/snap/bin/chromium",
            ];
            for c in candidates {
                let p = PathBuf::from(c);
                if p.exists() {
                    return Some(p);
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            let candidates = [
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            ];
            for c in candidates {
                let p = PathBuf::from(c);
                if p.exists() {
                    return Some(p);
                }
            }
        }

        None
    }

    /// Injects stealth anti-detection JavaScript into the active page context.
    #[instrument(skip(self))]
    pub async fn apply_stealth_scripts(&self) -> HarnessResult<()> {
        let stealth_js = r#"
            // Mask navigator.webdriver
            Object.defineProperty(navigator, 'webdriver', {
                get: () => undefined,
            });

            // Fake Chrome runtime object
            window.chrome = {
                app: {
                    isInstalled: false,
                    InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
                    RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' }
                },
                runtime: {
                    OnInstalledReason: { CHROME_UPDATE: 'chrome_update', INSTALL: 'install', SHARED_MODULE_UPDATE: 'shared_module_update', UPDATE: 'update' },
                    OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' },
                    PlatformArch: { ARM: 'arm', ARM64: 'arm64', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
                    PlatformNaclArch: { ARM: 'arm', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
                    PlatformOs: { ANDROID: 'android', CROS: 'cros', LINUX: 'linux', MAC: 'mac', OPENBSD: 'openbsd', WIN: 'win' },
                    RequestUpdateCheckStatus: { NO_UPDATE: 'no_update', THROTTLED: 'throttled', UPDATE_AVAILABLE: 'update_available' }
                }
            };

            // Mask plugins
            Object.defineProperty(navigator, 'plugins', {
                get: () => [1, 2, 3, 4, 5],
            });

            // Mask languages
            Object.defineProperty(navigator, 'languages', {
                get: () => ['en-US', 'en'],
            });

            // Mask WebGL vendor and renderer
            const getParameter = WebGLRenderingContext.prototype.getParameter;
            WebGLRenderingContext.prototype.getParameter = function(parameter) {
                if (parameter === 37445) {
                    return 'Intel Inc.';
                }
                if (parameter === 37446) {
                    return 'Intel(R) Iris(R) Xe Graphics';
                }
                return getParameter.apply(this, arguments);
            };
        "#;

        let page = self.active_page.lock().await;
        page.evaluate(stealth_js)
            .await
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to evaluate stealth scripts: {e}"),
            })?;

        debug!("applied stealth scripts to active page");
        Ok(())
    }

    /// Navigate active page to target URL and await network idle.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn goto(&self, url: &str) -> HarnessResult<()> {
        let page = self.active_page.lock().await;
        page.goto(url).await.map_err(|e| HarnessError::Http {
            reason: format!("navigation to '{url}' failed: {e}"),
        })?;

        // Give a short grace period for initial paint & DOM reconciliation.
        tokio::time::sleep(Duration::from_millis(150)).await;
        debug!(url, "navigation complete");
        Ok(())
    }

    /// Return the active page reference.
    pub async fn page(&self) -> Page {
        self.active_page.lock().await.clone()
    }

    /// Return the browser reference.
    pub fn browser(&self) -> Arc<Mutex<Browser>> {
        self.browser.clone()
    }

    // ── Synthetic Hardware Input (Bézier Mouse & Jitter) ────────────

    /// Move mouse from current position to (target_x, target_y) along a
    /// cubic Bézier trajectory with human micro-tremor jitter.
    #[instrument(skip(self))]
    pub async fn move_mouse_bezier(&self, target_x: f64, target_y: f64) -> HarnessResult<()> {
        let start = {
            let pos = self.current_mouse_pos.lock().await;
            *pos
        };

        let target = Point::new(target_x, target_y);
        let path = Self::calculate_bezier_points(start, target);

        let page = self.active_page.lock().await;

        for point in &path {
            let event = DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MouseMoved)
                .x(point.x)
                .y(point.y)
                .build()
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to construct MouseMoved event: {e}"),
                })?;

            page.execute(event)
                .await
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to dispatch MouseMoved event: {e}"),
                })?;

            // Micro-pacing delay: randomized 2ms to 7ms per interpolated step.
            let delay_ms = rand::thread_rng().gen_range(2..=7);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }

        // Update stored cursor position.
        let mut pos = self.current_mouse_pos.lock().await;
        *pos = target;

        debug!(
            from_x = start.x,
            from_y = start.y,
            to_x = target.x,
            to_y = target.y,
            steps = path.len(),
            "bezier mouse move completed"
        );
        Ok(())
    }

    /// Calculates a human-like cubic Bézier curve between start and target points.
    ///
    /// Formula:
    ///   B(t) = (1-t)³·P₀ + 3(1-t)²t·P₁ + 3(1-t)t²·P₂ + t³·P₃,  t ∈ [0, 1]
    ///
    /// Adds perpendicular drift and Gaussian micro-tremor to control points.
    pub fn calculate_bezier_points(p0: Point, p3: Point) -> Vec<Point> {
        let dx = p3.x - p0.x;
        let dy = p3.y - p0.y;
        let distance = (dx * dx + dy * dy).sqrt();

        // If distance is tiny, just return the destination.
        if distance < 1.0 {
            return vec![p3];
        }

        let mut rng = rand::thread_rng();

        // Perpendicular vector for hand drift.
        let perp_x = -dy / distance;
        let perp_y = dx / distance;

        // Dynamic perturbation factors.
        let drift1 = rng.gen_range(-0.25..0.25) * distance;
        let drift2 = rng.gen_range(-0.25..0.25) * distance;

        // Two dynamic control points along the 1/3 and 2/3 trajectory.
        let p1 = Point::new(
            p0.x + dx * 0.33 + perp_x * drift1,
            p0.y + dy * 0.33 + perp_y * drift1,
        );
        let p2 = Point::new(
            p0.x + dx * 0.66 + perp_x * drift2,
            p0.y + dy * 0.66 + perp_y * drift2,
        );

        // Micro-steps: 15 to 25 interpolated points based on distance.
        let steps = rng.gen_range(15..=25);
        let mut points = Vec::with_capacity(steps);

        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let one_minus_t = 1.0 - t;

            // Cubic Bézier calculation
            let x = one_minus_t.powi(3) * p0.x
                + 3.0 * one_minus_t.powi(2) * t * p1.x
                + 3.0 * one_minus_t * t.powi(2) * p2.x
                + t.powi(3) * p3.x;

            let y = one_minus_t.powi(3) * p0.y
                + 3.0 * one_minus_t.powi(2) * t * p1.y
                + 3.0 * one_minus_t * t.powi(2) * p2.y
                + t.powi(3) * p3.y;

            // Add subtle micro-jitter (human hand tremor) ±0.5px.
            let jitter_x = rng.gen_range(-0.5..=0.5);
            let jitter_y = rng.gen_range(-0.5..=0.5);

            points.push(Point::new(x + jitter_x, y + jitter_y));
        }

        // Guarantee final point matches exact destination.
        if let Some(last) = points.last_mut() {
            *last = p3;
        }

        points
    }

    /// Dispatch trusted click sequence:
    ///  1. Move mouse via Bézier curve to target.
    ///  2. Dispatch `MousePressed`.
    ///  3. Dwell for 40ms - 110ms.
    ///  4. Dispatch `MouseReleased`.
    #[instrument(skip(self))]
    pub async fn click_at(&self, target_x: f64, target_y: f64) -> HarnessResult<()> {
        // Step 1: Smooth Bézier approach.
        self.move_mouse_bezier(target_x, target_y).await?;

        let page = self.active_page.lock().await;

        // Step 2: MousePressed
        let press = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MousePressed)
            .x(target_x)
            .y(target_y)
            .button(MouseButton::Left)
            .click_count(1)
            .build()
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to build MousePressed: {e}"),
            })?;

        page.execute(press)
            .await
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to dispatch MousePressed: {e}"),
            })?;

        // Step 3: Human dwell time: 40ms to 110ms.
        let dwell_ms = rand::thread_rng().gen_range(40..=110);
        tokio::time::sleep(Duration::from_millis(dwell_ms)).await;

        // Step 4: MouseReleased
        let release = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .x(target_x)
            .y(target_y)
            .button(MouseButton::Left)
            .click_count(1)
            .build()
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to build MouseReleased: {e}"),
            })?;

        page.execute(release)
            .await
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to dispatch MouseReleased: {e}"),
            })?;

        debug!(target_x, target_y, dwell_ms, "click sequence dispatched");
        Ok(())
    }

    /// Types text into the currently focused element with randomized inter-keystroke intervals.
    #[instrument(skip(self, text))]
    pub async fn type_text(&self, text: &str) -> HarnessResult<()> {
        let page = self.active_page.lock().await;

        for ch in text.chars() {
            let ch_str = ch.to_string();

            // KeyDown / RawKeyDown
            let key_down = DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .text(ch_str.clone())
                .unmodified_text(ch_str.clone())
                .build()
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to build KeyDown: {e}"),
                })?;
            page.execute(key_down)
                .await
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to dispatch KeyDown: {e}"),
                })?;

            // KeyUp
            let key_up = DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .build()
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to build KeyUp: {e}"),
                })?;
            page.execute(key_up)
                .await
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to dispatch KeyUp: {e}"),
                })?;

            // Realistic typing interval: 20ms - 65ms per keystroke.
            let delay_ms = rand::thread_rng().gen_range(20..=65);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }

        debug!(chars = text.len(), "keyboard typing sequence completed");
        Ok(())
    }

    /// Scroll page vertically by `delta_y` and horizontally by `delta_x`.
    #[instrument(skip(self))]
    pub async fn scroll_by(&self, delta_x: f64, delta_y: f64) -> HarnessResult<()> {
        let pos = {
            let p = self.current_mouse_pos.lock().await;
            *p
        };

        let page = self.active_page.lock().await;

        let wheel = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseWheel)
            .x(pos.x)
            .y(pos.y)
            .delta_x(delta_x)
            .delta_y(delta_y)
            .build()
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to build MouseWheel: {e}"),
            })?;

        page.execute(wheel)
            .await
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to dispatch MouseWheel: {e}"),
            })?;

        tokio::time::sleep(Duration::from_millis(50)).await;
        debug!(delta_x, delta_y, "scroll event dispatched");
        Ok(())
    }

    // ── Session Cookie Extraction & Injection ───────────────────────

    /// Extract all authenticated session cookies from the browser context.
    #[instrument(skip(self))]
    pub async fn get_cookies(&self) -> HarnessResult<Vec<CookieEntry>> {
        let page = self.active_page.lock().await;
        let res = page
            .execute(GetCookiesParams::default())
            .await
            .map_err(|e| HarnessError::CdpConnection {
                reason: format!("GetCookies CDP call failed: {e}"),
            })?;

        let mut entries = Vec::with_capacity(res.cookies.len());
        for c in &res.cookies {
            entries.push(CookieEntry {
                name: c.name.clone(),
                value: c.value.clone(),
                domain: c.domain.clone(),
                path: c.path.clone(),
                secure: c.secure,
                http_only: c.http_only,
                expires: Some(c.expires),
            });
        }

        debug!(count = entries.len(), "cookies retrieved");
        Ok(entries)
    }

    /// Inject cookies into the active browser context.
    #[instrument(skip(self, cookies))]
    pub async fn set_cookies(&self, cookies: &[CookieEntry]) -> HarnessResult<()> {
        let page = self.active_page.lock().await;

        for c in cookies {
            let mut builder = SetCookieParams::builder()
                .name(c.name.clone())
                .value(c.value.clone())
                .domain(c.domain.clone())
                .path(c.path.clone())
                .secure(c.secure)
                .http_only(c.http_only);

            if let Some(exp) = c.expires {
                builder = builder.expires(TimeSinceEpoch::new(exp));
            }

            let param = builder.build().map_err(|e| HarnessError::CdpConnection {
                reason: format!("failed to build SetCookieParams: {e}"),
            })?;

            page.execute(param)
                .await
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("SetCookie CDP call failed for '{}': {e}", c.name),
                })?;
        }

        debug!(count = cookies.len(), "cookies injected");
        Ok(())
    }

    // ── HITL Mode Transition ────────────────────────────────────────

    /// Returns whether the driver is currently operating in headless mode.
    pub fn is_headless(&self) -> bool {
        self.is_headless.load(Ordering::Relaxed)
    }

    /// Switch execution context to a visible headed browser window for HITL.
    ///
    /// Preserves current cookies and navigates the headed window to `current_url`.
    #[instrument(skip(self))]
    pub async fn switch_to_headed(&self) -> HarnessResult<()> {
        if !self.is_headless() {
            warn!("browser is already in headed mode");
            return Ok(());
        }

        info!("HITL handoff initiated: switching to headed browser window");

        // 1. Snapshot current URL and cookies.
        let current_url = {
            let page = self.active_page.lock().await;
            page.url()
                .await
                .map_err(|e| HarnessError::CdpConnection {
                    reason: format!("failed to get current URL: {e}"),
                })?
                .unwrap_or_else(|| "about:blank".to_string())
        };
        let cookies = self.get_cookies().await?;

        // 2. Spawn a new headed browser instance.
        let mut headed_config = self.config.clone();
        headed_config.headless = false;

        let (new_browser, new_page) = Self::spawn_chromium(&headed_config).await?;

        // 3. Swap references atomically under lock.
        {
            let mut browser_guard = self.browser.lock().await;
            let mut page_guard = self.active_page.lock().await;
            *browser_guard = new_browser;
            *page_guard = new_page;
            self.is_headless.store(false, Ordering::SeqCst);
        }

        // 4. Inject cookies and navigate to current URL.
        self.apply_stealth_scripts().await?;
        self.set_cookies(&cookies).await?;
        self.goto(&current_url).await?;

        info!("successfully transitioned to headed browser for human interaction");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bezier_points_count_and_continuity() {
        let p0 = Point::new(100.0, 100.0);
        let p3 = Point::new(500.0, 400.0);

        let points = BrowserDriver::calculate_bezier_points(p0, p3);

        // Verify bounds on number of steps (15..=25)
        assert!(points.len() >= 15 && points.len() <= 25);

        // Verify exact arrival at target
        let last = points.last().unwrap();
        assert_eq!(last.x, 500.0);
        assert_eq!(last.y, 400.0);

        // Verify no NaN or infinite coordinates
        for pt in &points {
            assert!(pt.x.is_finite());
            assert!(pt.y.is_finite());
        }
    }

    #[test]
    fn test_bezier_short_distance() {
        let p0 = Point::new(200.0, 200.0);
        let p3 = Point::new(200.4, 200.3); // Distance < 1.0

        let points = BrowserDriver::calculate_bezier_points(p0, p3);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0], p3);
    }

    #[tokio::test]
    #[ignore = "requires live Chromium binary and unsandboxed OS permissions"]
    async fn test_browser_launch() {
        let config = BrowserDriverConfig::default();
        let res = BrowserDriver::launch(config).await;
        println!("Launch result: {:?}", res.as_ref().map(|_| ()));
        if let Err(e) = res {
            panic!("Browser launch failed: {e}");
        }
    }
}
