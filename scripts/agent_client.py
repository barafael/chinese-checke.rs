#!/usr/bin/env python3
"""Drive a running Chinese Checkers window like an agent would.

Connects to the game's agent interface (localhost TCP, JSON lines — see
`checkers-bevy/src/agent.rs`), then runs the look-act-check loop: screenshot,
click, key, and state, with no human input device involved.

Start the game, note the `agent port N` line it prints, then:

    python3 scripts/agent_client.py --port N [--png shot.png] [command ...]

Commands (each is one request):
    shot FILE          save a screenshot as PNG
    state              print the game state as JSON
    click X Y          click at logical pixel coordinates
    move X Y           move the cursor
    key NAME           press a key (enter, escape, a, ...)
    text STRING        type a string
    resize W H         set the window size
    click-hole Q R     click the board hole at hex coordinates (q, r)

    demo               play one move solo: click a piece, click a
                       destination, press Enter — the whole loop a person
                       would run, read from `state` rather than pixels.

With no commands, `demo` runs.

Coordinate note: `click`/`move` take *logical* pixels, origin at the window's
top-left — the same numbers the `window` command reports. `click-hole` does
the board math for you from the game's own mapping.
"""

import argparse
import base64
import json
import socket
import sys
import time

DEFAULT_PORT = 4310


class Agent:
    """One connection to the agent interface: request in, response out."""

    def __init__(self, port: int, host: str = "127.0.0.1"):
        self.sock = socket.create_connection((host, port), timeout=30)
        self.f = self.sock.makefile("rwb")

    def ask(self, **cmd) -> dict:
        self.f.write((json.dumps(cmd) + "\n").encode())
        self.f.flush()
        line = self.f.readline()
        if not line:
            raise ConnectionError("the game closed the connection")
        reply = json.loads(line)
        if not reply.get("ok"):
            raise RuntimeError(f"{cmd.get('cmd')}: {reply.get('error')}")
        return reply

    def screenshot(self) -> bytes:
        reply = self.ask(cmd="screenshot")
        return base64.b64decode(reply["png"])

    def click(self, x: float, y: float):
        self.ask(cmd="click", x=x, y=y)

    def key(self, name: str, shift: bool = False):
        self.ask(cmd="key", key=name, shift=shift)

    def text(self, s: str):
        self.ask(cmd="text", text=s)

    def state(self) -> dict:
        return self.ask(cmd="state")["state"]


def hole_pixel(state: dict, q: int, r: int) -> tuple[float, float]:
    """Screen position of hole (q, r), from the game's own board mapping.

    The board is centred; hole spacing is 34 world units, pointy-top axial,
    and the camera guarantees the whole board fits the window (scaled to
    whichever axis is binding). This mirrors
    `checkers-bevy/src/board_view.rs` plus the `AutoMin` camera fit.
    """
    spacing = 34.0
    frame_w = (204.0 + 13.0) * 2 * 1.12
    frame_h = (236.0 + 13.0) * 2 * 1.12

    win = agent_window(state)
    scale = min(win["width"] / frame_w, win["height"] / frame_h)
    x = spacing * (q + r / 2)
    y = -spacing * (3**0.5) / 2 * r
    return (
        win["width"] / 2 + x * scale,
        win["height"] / 2 - y * scale,
    )


def agent_window(state: dict) -> dict:
    """The window command's answer, cached on the state dict by the caller."""
    return state["_window"]


def button(state: dict, label: str, contains: bool = False) -> tuple[float, float]:
    """Centre of the first button whose label matches, from `state`.

    `contains` matches substrings, so "Human" finds "Human (1)" too —
    labels often carry the key that does the same thing.
    """
    for b in state["buttons"]:
        text = b.get("label") or ""
        if text == label or (contains and label in text):
            return b["x"], b["y"]
    raise LookupError(f"no button labelled {label!r}; on screen: "
                      f"{[b.get('label') for b in state['buttons']]}")


def fresh_state(agent: Agent) -> dict:
    state = agent.state()
    state["_window"] = agent.ask(cmd="window")
    return state


def configure_solo(agent: Agent, corners=(0, 3)) -> None:
    """Fill the table from the lobby: select each corner, press its Human
    button — the same two presses a person makes."""
    for corner in corners:
        agent.key(str(corner + 1))  # digits select corners, 1-based
        state = fresh_state(agent)
        human = f"Human ({corner + 1})" if corner else "Human"
        try:
            agent.click(*button(state, str(corner + 1) + ": Human", contains=True))
        except LookupError:
            # Labels differ per layout; fall back to any Human button.
            agent.click(*button(state, "Human", contains=True))
        state = fresh_state(agent)
        print("  table:", state["lobby"]["table"], "-", state["lobby"]["status"])


def demo(agent: Agent) -> None:
    """From a cold lobby to one played move, decided from `state` alone."""
    state = fresh_state(agent)
    if state["phase"] == "Lobby":
        print("in the lobby:", state["lobby"]["status"] or "(ready)")
        configure_solo(agent)
        agent.key("enter")
        time.sleep(0.5)
        state = fresh_state(agent)
        print("phase:", state["phase"], "-", state["lobby"]["status"])

    if state["phase"] != "InGame":
        raise SystemExit(f"still in {state['phase']}: {state['lobby']['status']}")

    print("board pieces:", len(state["board"]), "| turn:", state["turn"])
    turn = state["turn"]
    mine = [[q, r] for q, r, p in state["board"] if p == turn]

    for q, r in mine:
        agent.click(*hole_pixel(state, q, r))
        state = fresh_state(agent)
        if state["selection"]["kind"] == "piece":
            print(f"selected ({q},{r}) -> {len(state['highlights'])} destination(s)")
            break
    else:
        raise SystemExit("no piece offered a destination")

    if not state["highlights"]:
        raise SystemExit("selected piece has nowhere to go")

    q, r = state["highlights"][0]
    agent.click(*hole_pixel(state, q, r))
    agent.key("enter")
    time.sleep(0.5)
    final = fresh_state(agent)
    print("after move:", final["message"], "| moves:", final["moves_committed"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--png", metavar="FILE", help="with `shot`: where to write")
    parser.add_argument("command", nargs="*", help="see the module docstring")
    args = parser.parse_args()

    agent = Agent(args.port)

    words = args.command or ["demo"]
    match words:
        case ["shot"]:
            path = args.png or "screenshot.png"
            with open(path, "wb") as f:
                f.write(agent.screenshot())
            print(f"wrote {path}")
        case ["state"]:
            print(json.dumps(agent.state(), indent=2))
        case ["click", x, y]:
            agent.click(float(x), float(y))
        case ["move", x, y]:
            agent.ask(cmd="move", x=float(x), y=float(y))
        case ["key", name]:
            agent.key(name)
        case ["text", s]:
            agent.text(s)
        case ["resize", w, h]:
            agent.ask(cmd="resize", width=float(w), height=float(h))
        case ["click-hole", q, r]:
            state = fresh_state(agent)
            agent.click(*hole_pixel(state, int(q), int(r)))
        case ["button", *label]:
            agent.click(*button(fresh_state(agent), " ".join(label), contains=True))
        case ["demo"]:
            demo(agent)
        case _:
            parser.error("unknown command " + " ".join(words))
            return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
