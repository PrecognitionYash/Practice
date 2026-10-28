// ──────────────────────────────────────────────────────────────────────
// harness-tui/src/events.rs — Glitch-Free RAII Terminal Guard & Event Stream
//
// Implements:
//  • RAII TerminalGuard with panic hook to guarantee screen buffer restoration
//  • 60 FPS tick event stream (16.6ms cadence) for double-buffered ANSI diffing
//  • Non-blocking asynchronous input event processing
// ──────────────────────────────────────────────────────────────────────

use std::io::{stdout, Stdout};
use std::panic;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyEvent, MouseEvent,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use harness_core::error::{HarnessError, HarnessResult};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tracing::info;

/// Target frame duration locked to 60 FPS (16.66ms).
pub const FRAME_DURATION: Duration = Duration::from_millis(16);

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// 60 FPS frame tick for double-buffered differential rendering.
    Tick,
    /// Terminal keyboard input event.
    Key(KeyEvent),
    /// Terminal mouse event (wheel scrolls and clicks).
    Mouse(MouseEvent),
    /// Terminal window resize event.
    Resize(u16, u16),
}

/// RAII Terminal Guard ensuring the terminal raw mode and alternate screen buffer
/// are restored under all conditions (clean exit, error, or panic).
pub struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    /// Initialize the terminal into raw mode and switch to alternate screen with mouse capture.
    pub fn new() -> HarnessResult<Self> {
        enable_raw_mode().map_err(HarnessError::Io)?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, EnableMouseCapture).map_err(HarnessError::Io)?;

        // Set panic hook to restore terminal and mouse capture on panic
        let orig_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
            orig_hook(panic_info);
        }));

        let backend = CrosstermBackend::new(out);
        let terminal = Terminal::new(backend).map_err(HarnessError::Io)?;

        info!("RAII TerminalGuard initialized (raw mode + alternate screen + mouse capture)");
        Ok(Self { terminal })
    }

    /// Access mutable reference to the underlying Ratatui terminal.
    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
        info!("RAII TerminalGuard dropped: restored terminal buffer & mouse capture");
    }
}

/// Event stream that multiplexes crossterm keyboard and mouse inputs with 60 FPS render ticks.
pub struct EventStream {
    rx: mpsc::Receiver<AppEvent>,
}

impl EventStream {
    /// Start the background event reader task.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(128);

        // Spawn background blocking task to read crossterm events
        let event_tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            loop {
                // Poll for crossterm events with a short timeout
                if event::poll(Duration::from_millis(10)).unwrap_or(false) {
                    if let Ok(crossterm_event) = event::read() {
                        match crossterm_event {
                            CrosstermEvent::Key(key) => {
                                if event_tx.blocking_send(AppEvent::Key(key)).is_err() {
                                    break;
                                }
                            }
                            CrosstermEvent::Mouse(mouse) => {
                                if event_tx.blocking_send(AppEvent::Mouse(mouse)).is_err() {
                                    break;
                                }
                            }
                            CrosstermEvent::Resize(w, h)
                                if event_tx.blocking_send(AppEvent::Resize(w, h)).is_err() =>
                            {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        // Spawn 60 FPS tick timer task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(FRAME_DURATION);
            loop {
                interval.tick().await;
                if tx.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        });

        Self { rx }
    }

    /// Receive the next event from the stream.
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}
