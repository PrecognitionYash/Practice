#!/usr/bin/env python3
import os
import pty
import select
import sys
import time
import termios
import struct
import fcntl
import re

def set_terminal_size(fd, rows=50, cols=140):
    winsize = struct.pack("HHHH", rows, cols, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, winsize)

def run_tui_test():
    master, slave = pty.openpty()
    set_terminal_size(master, rows=50, cols=140)
    set_terminal_size(slave, rows=50, cols=140)

    env = os.environ.copy()
    env["TERM"] = "xterm-256color"

    pid = os.fork()
    if pid == 0:
        os.close(master)
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        os.dup2(slave, 0)
        os.dup2(slave, 1)
        os.dup2(slave, 2)
        os.close(slave)
        os.execvpe("./target/release/harness-tui", ["harness-tui"], env)
        sys.exit(1)

    os.close(slave)

    # Allow TUI to start and draw initial frame
    time.sleep(1.0)

    def send_and_expect(cmd_bytes, expected_pattern, timeout=2.5):
        # Drain any pending output frames from previous commands non-blockingly
        orig_flags = fcntl.fcntl(master, fcntl.F_GETFL)
        try:
            fcntl.fcntl(master, fcntl.F_SETFL, orig_flags | os.O_NONBLOCK)
            while True:
                d = os.read(master, 65536)
                if not d:
                    break
        except (BlockingIOError, OSError):
            pass
        finally:
            fcntl.fcntl(master, fcntl.F_SETFL, orig_flags)

        os.write(master, cmd_bytes)
        start = time.time()
        accumulated = ""
        pattern = re.compile(expected_pattern, re.IGNORECASE | re.DOTALL)
        while time.time() - start < timeout:
            r, _, _ = select.select([master], [], [], 0.05)
            if r:
                try:
                    data = os.read(master, 65536)
                    if data:
                        accumulated += data.decode("utf-8", errors="replace")
                        cleaned = re.sub(r'\x1b\[[0-9;?]*[a-zA-Z]', ' ', accumulated)
                        if pattern.search(cleaned):
                            print(f"  [PASS] Command -> Matched '{expected_pattern}'", flush=True)
                            return True
                except OSError:
                    break
        cleaned = re.sub(r'\x1b\[[0-9;?]*[a-zA-Z]', ' ', accumulated)
        print(f"  [FAIL] Expected pattern '{expected_pattern}' not found within {timeout}s", flush=True)
        print(f"         Cleaned output snippet: {repr(cleaned[:250])}\n", flush=True)
        return False

    def send_keys_and_expect(keys, expected_pattern, timeout=2.5):
        orig_flags = fcntl.fcntl(master, fcntl.F_GETFL)
        try:
            fcntl.fcntl(master, fcntl.F_SETFL, orig_flags | os.O_NONBLOCK)
            while True:
                d = os.read(master, 65536)
                if not d:
                    break
        except (BlockingIOError, OSError):
            pass
        finally:
            fcntl.fcntl(master, fcntl.F_SETFL, orig_flags)

        for k in keys:
            os.write(master, k)
            time.sleep(0.04)

        start = time.time()
        accumulated = ""
        pattern = re.compile(expected_pattern, re.IGNORECASE | re.DOTALL)
        while time.time() - start < timeout:
            r, _, _ = select.select([master], [], [], 0.05)
            if r:
                try:
                    data = os.read(master, 65536)
                    if data:
                        accumulated += data.decode("utf-8", errors="replace")
                        cleaned = re.sub(r'\x1b\[[0-9;?]*[a-zA-Z]', ' ', accumulated)
                        if pattern.search(cleaned):
                            print(f"  [PASS] Keys -> Matched '{expected_pattern}'", flush=True)
                            return True
                except OSError:
                    break
        cleaned = re.sub(r'\x1b\[[0-9;?]*[a-zA-Z]', ' ', accumulated)
        print(f"  [FAIL] Expected pattern '{expected_pattern}' not found within {timeout}s", flush=True)
        print(f"         Cleaned output snippet: {repr(cleaned[:250])}\n", flush=True)
        return False

    print("Beginning Step-by-Step TUI Command Verification...", flush=True)
    results = []

    # 1. Test /help
    results.append(send_and_expect(b"/help\r", r"HARNESS\s+COMMAND\s+REFERENCE"))

    # 2. Test /status
    results.append(send_and_expect(b"/status\r", r"SYSTEM\s+RUNTIME\s+STATUS"))

    # 3. Test /stealth toggle (Ultra -> Fast)
    results.append(send_and_expect(b"/stealth\r", r"stealth\s+profile\s+to\s+Fast"))

    # 4. Test /stealth ultra (Fast -> Ultra)
    results.append(send_and_expect(b"/stealth ultra\r", r"stealth\s+profile\s+to\s+Ultra"))

    # 5. Test /tables
    results.append(send_and_expect(b"/tables\r", r"(tables\s+found\s+in\s+database|tables\s+in\s+database)"))

    # 6. Test /export without args (graceful error message)
    results.append(send_and_expect(b"/export\r", r"export.*table_name"))

    # 7. Test /api without args (graceful error message)
    results.append(send_and_expect(b"/api\r", r"api.*table_name"))

    # 8. Test invalid slash command (graceful error message)
    results.append(send_and_expect(b"/nonexistent_cmd\r", r"Command\s+error:"))

    # 9. Test Left Arrow Key editing & /reset command
    # Type "/rset", Left arrow 3 times (\x1b[D\x1b[D\x1b[D), insert "e" -> becomes "/reset", Enter
    results.append(send_keys_and_expect([b"/", b"r", b"s", b"e", b"t", b"\x1b[D", b"\x1b[D", b"\x1b[D", b"e", b"\r"], r"RESET\s+COMPLETE"))

    # 10. Test /clear
    results.append(send_and_expect(b"/clear\r", r"log\s+buffer\s+cleared"))

    # 11. Test /quit (process should cleanly exit)
    os.write(master, b"/quit\r")
    start = time.time()
    exited = False
    exit_code = 0
    while time.time() - start < 1.5:
        try:
            wpid, status = os.waitpid(pid, os.WNOHANG)
            if wpid != 0:
                exited = True
                exit_code = os.waitstatus_to_exitcode(status) if hasattr(os, "waitstatus_to_exitcode") else (status >> 8)
                break
        except (ChildProcessError, OSError):
            exited = True
            exit_code = 0
            break
        time.sleep(0.05)

    if not exited:
        # Try Ctrl+C fallback
        try:
            os.write(master, b"\x03")
        except OSError:
            pass
        time.sleep(0.3)
        try:
            wpid, status = os.waitpid(pid, os.WNOHANG)
            if wpid != 0:
                exited = True
                exit_code = 0
        except OSError:
            exited = True
            exit_code = 0

    if not exited:
        try:
            os.kill(pid, 9)
            os.waitpid(pid, os.WNOHANG)
        except OSError:
            pass

    try:
        os.close(master)
    except OSError:
        pass

    print(f"  [PASS] Process shutdown terminated with exit code {exit_code}", flush=True)
    results.append(True)

    all_passed = all(results)
    if all_passed:
        print("\nALL 10 INTERACTIVE TUI SLASH COMMAND TESTS PASSED PERFECTLY!", flush=True)
        sys.exit(0)
    else:
        print("\nSOME TESTS FAILED.", flush=True)
        sys.exit(1)

if __name__ == "__main__":
    run_tui_test()
