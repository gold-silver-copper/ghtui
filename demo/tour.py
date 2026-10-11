#!/usr/bin/env python3
"""A scripted tour of ghtui, for recording (see record.sh, which runs this in foot).

ghtui runs in a pseudo-terminal of ours: what it draws goes through to the terminal as is,
what the terminal answers goes back to it, and the tour types keys in between. Each step
waits for ghtui to have drawn what it's waiting for (ghtui writes every frame's text to
GHTUI_FRAME_DUMP), so a slow answer from GitHub slows the tour down instead of breaking it.

Everything it opens is public, and it only reads: it never merges, approves, comments or
stars. Each scene and key is written to $GHTUI_DEMO_LOG as `<unix time>\t<kind>\t<text>`,
for the captions (captions.py).
"""

import fcntl
import os
import pty
import select
import signal
import subprocess
import sys
import termios
import threading
import time
import tty

GHTUI = os.environ.get("GHTUI_BIN", "ghtui")
FRAME = os.environ["GHTUI_FRAME_DUMP"]
LOG = os.environ.get("GHTUI_DEMO_LOG")
# A screenshot at the end of each scene goes here, for checking the tour.
SHOTS = os.environ.get("GHTUI_DEMO_SHOTS")
# Slower or faster overall (2 = twice as slow).
PACE = float(os.environ.get("GHTUI_DEMO_PACE", "0.8"))
# Touched by record.sh once it has stopped recording: only then does the tour quit
# ghtui, so its window never closes on camera.
DONE = os.environ.get("GHTUI_DEMO_DONE")

KEYS = {
    "enter": "\r",
    "esc": "\x1b",
    "tab": "\t",
    "up": "\x1b[A",
    "down": "\x1b[B",
    "right": "\x1b[C",
    "left": "\x1b[D",
    "sdown": "\x1b[1;2B",
    "sup": "\x1b[1;2A",
    "pgdn": "\x1b[6~",
    "pgup": "\x1b[5~",
    "space": " ",
    "bs": "\x7f",
    "ctrl-w": "\x17",
    "ctrl-u": "\x15",
}

# What the tour shows: public, settled, and worth looking at.
REPO = "ratatui/ratatui"
# A merged refactor across ten files: changed words within lines, scope names in hunk
# headers, and a new file.
PR = "ratatui/ratatui#2400"
# A failed Clippy run: its log ends in Rust's own errors. (GitHub keeps logs 90 days.)
JOB = "https://github.com/ratatui/ratatui/actions/runs/37571003396/job/112629378439"
PROFILE = "@joshka"
SEARCH = "terminal file manager"


class Tour:
    def __init__(self, fd):
        self.fd = fd

    def pause(self, secs):
        time.sleep(secs * PACE)

    def log(self, kind, text=""):
        if LOG:
            with open(LOG, "a") as f:
                f.write(f"{time.time():.3f}\t{kind}\t{text}\n")

    def scene(self, caption):
        """A new part of the tour: its caption from now on."""
        if SHOTS:
            n = len(os.listdir(SHOTS))
            subprocess.run(["grim", "-o", os.environ.get("GHTUI_DEMO_OUTPUT", ""), f"{SHOTS}/{n:02}.png"], check=False)
        self.log("scene", caption)

    def key(self, *names, gap=0.45):
        """Press keys: names from KEYS, or single characters."""
        for name in names:
            self.log("key", name)
            os.write(self.fd, KEYS.get(name, name).encode())
            self.pause(gap)

    def type(self, text, gap=0.07):
        self.log("type", text)
        for c in text:
            os.write(self.fd, c.encode())
            time.sleep(gap * PACE)
        self.pause(0.3)

    def go(self, where, *wait, timeout=60.0):
        """Opens `where` from the command palette, and waits for what it shows."""
        self.key(":")
        self.type(where)
        self.key("enter", gap=0.1)
        self.wait_for(*wait, timeout=timeout)

    def screen(self):
        try:
            with open(FRAME, encoding="utf-8", errors="replace") as f:
                return f.read()
        except OSError:
            return ""

    def wait_for(self, *texts, timeout=30.0, gone=False):
        """Until the screen shows any of `texts` (or, with `gone`, none of them). The wait
        is logged, and cut from the video (captions.py): it's the network, not ghtui."""
        self.log("hold")
        end = time.time() + timeout
        found = False
        while time.time() < end:
            s = self.screen()
            if any(t in s for t in texts) != gone:
                found = True
                break
            time.sleep(0.05)
        # A moment for what came to be drawn.
        time.sleep(0.4)
        self.log("go")
        return found


def tour(t):
    # The video starts with the first scene: by then the pointer Hyprland draws on a new
    # monitor has timed out, and Home's sections are in.
    t.wait_for("ratatui: open pull requests", timeout=30)
    t.wait_for("Loading", gone=True, timeout=30)
    time.sleep(3)
    t.scene("ghtui: GitHub in your terminal. Home is your own saved searches")
    t.pause(3)
    t.key("j", "j", "j", gap=0.6)
    t.key("pgdn", gap=1.2)
    t.key("pgdn", gap=1.2)
    t.pause(1)

    t.scene(": goes anywhere: a repository, an issue or pull request, any github.com URL")
    t.go(PR, "merged into", "Conversation")
    t.pause(2.5)
    t.key("j", "j", "j", gap=0.8)
    t.pause(1)

    t.scene("A pull request's tabs: gt and gT go through its commits, checks and files")
    t.key("gt")
    t.wait_for("Commits", timeout=20)
    t.pause(2)
    t.key("gt")
    t.wait_for("Continuous Integration", timeout=20)
    t.pause(2.5)

    t.scene("Files changed: diffed locally with git, highlighted, changed words picked out")
    t.key("gt")
    t.wait_for("@@", timeout=120)
    t.pause(3)
    t.key("]c", gap=1.4)
    t.key("]c", gap=1.4)
    t.key("]c", gap=1.4)
    t.pause(1)

    t.scene("S: split view, old and new side by side")
    t.key("S")
    t.pause(3)
    t.key("]c", gap=1.4)
    t.key("]c", gap=1.4)
    t.pause(1)

    t.scene("]f goes file by file; the tree on the left follows")
    t.key("]f", gap=1.6)
    t.key("]f", gap=1.6)
    t.key("]f", gap=1.6)
    t.pause(1)

    t.scene("/ searches the diff; n goes to the next match")
    t.key("/")
    t.type("cell_width")
    t.key("enter")
    t.pause(1.5)
    t.key("n", gap=1.2)
    t.key("n", gap=1.2)
    t.key("esc")
    t.key("S")
    t.pause(1)

    t.scene("space lists what you can do here, and says why when you can't")
    t.key("space")
    t.pause(4)
    t.key("esc")
    t.pause(0.8)

    t.scene("A repository: files, README and About, as on GitHub")
    t.go(REPO, "About")
    t.pause(2.5)
    t.key("pgdn", gap=1.5)
    t.key("pgdn", gap=1.5)
    t.pause(1)

    t.scene("i puts letters on every link on screen: type them to follow one")
    t.key("i")
    t.pause(3)
    t.key("esc")
    t.key("gg")
    t.pause(0.8)

    t.scene("gt goes through a repository's tabs: issues, pull requests, Actions…")
    t.key("gt")
    t.wait_for("Open", timeout=20)
    t.pause(2)
    t.key("gt")
    t.wait_for("Open", timeout=20)
    t.pause(2)
    t.key("j", "j", gap=0.6)
    t.pause(1)

    t.scene("A failed job opens at its first error, its step's name on top")
    t.go(JOB, "error", timeout=60)
    t.pause(4)
    t.key("pgdn", gap=1.5)
    t.pause(1)

    t.scene("/ searches the log; n and N go from match to match")
    t.key("/")
    t.type("warning")
    t.key("enter")
    t.pause(1.5)
    t.key("n", gap=1.2)
    t.key("n", gap=1.2)
    t.key("esc")
    t.pause(1)

    t.scene("Tabs, like a browser's: T opens one, [b and ]b switch, ctrl-w closes")
    t.key("T")
    t.pause(1)
    t.go(PROFILE, "contributions", "Overview")
    t.pause(3)
    t.key("[b")
    t.pause(2)
    t.key("]b")
    t.pause(1.5)

    t.scene("Profiles: README, contributions, repositories, stars")
    t.key("pgdn", gap=1.5)
    t.key("gt")
    t.wait_for("Repositories", timeout=20)
    t.pause(2.5)
    t.key("ctrl-w")
    t.pause(1)

    t.scene("g/ searches GitHub: repositories, issues, pull requests, people, code; 1gt–7gt pick the tab")
    t.key("gh")
    t.wait_for("ratatui: open pull requests", timeout=20)
    t.key("g/")
    t.type(SEARCH)
    t.key("enter", gap=0.1)
    t.wait_for("results", timeout=30)
    t.pause(2)
    t.key("1gt")
    t.wait_for("repository results", timeout=20)
    t.pause(2.5)
    t.key("j", "j", gap=0.6)
    t.pause(1)

    t.scene(": is also the command palette: every action, by name")
    t.key(":")
    t.type("split")
    t.pause(2)
    t.key("ctrl-u")
    t.type("theme")
    t.pause(2)
    t.key("esc")
    t.pause(1)

    t.scene("ghtui: cargo install ghtui")
    t.pause(3)
    t.log("end")
    while DONE and not os.path.exists(DONE):
        time.sleep(0.1)
    t.key("ZZ")


def main():
    # The terminal's size, once the window has settled (it's made fullscreen as it opens).
    time.sleep(1.0)
    size = fcntl.ioctl(sys.stdin.fileno(), termios.TIOCGWINSZ, b"\0" * 8)
    # The token record.sh left for ghtui, read and removed.
    if token := os.environ.pop("GHTUI_DEMO_TOKEN", None):
        with open(token) as f:
            os.environ["GH_TOKEN"] = f.read().strip()
        os.remove(token)
    pid, fd = pty.fork()
    if pid == 0:
        os.execvp(GHTUI, [GHTUI, *sys.argv[1:]])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, size)

    def resized(*_):
        fcntl.ioctl(fd, termios.TIOCSWINSZ, fcntl.ioctl(sys.stdin.fileno(), termios.TIOCGWINSZ, b"\0" * 8))
        os.kill(pid, signal.SIGWINCH)

    signal.signal(signal.SIGWINCH, resized)
    old = termios.tcgetattr(sys.stdin.fileno())
    tty.setraw(sys.stdin.fileno())
    t = Tour(fd)
    threading.Thread(target=lambda: tour(t), daemon=True).start()
    try:
        while True:
            try:
                ready, _, _ = select.select([sys.stdin.fileno(), fd], [], [])
            except InterruptedError:
                continue
            if fd in ready:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                os.write(sys.stdout.fileno(), data)
            if sys.stdin.fileno() in ready:
                # The terminal's answers to ghtui's questions (its background color).
                os.write(fd, os.read(sys.stdin.fileno(), 4096))
    finally:
        termios.tcsetattr(sys.stdin.fileno(), termios.TCSADRAIN, old)
        os.waitpid(pid, 0)


if __name__ == "__main__":
    main()
