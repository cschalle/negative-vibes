#!/usr/bin/env python3
"""Failure-path and lifecycle checks for the prototype scanner portal.

Run through run-scenarios.sh, which starts a private session bus, the service and the
SANE `test` backend. These are the cases a pipe-and-signal API can get wrong: what the
app sees when it cancels, closes the pipe, vanishes, or asks for what it was not given.
"""
import sys
import time
import os

from gi.repository import GLib

from portal_client import IFACE, NAME, PATH, Client, check, failures, v


def main():
    c = Client()
    code, results = c.select(multiple=v("b", False))
    scanners = results["scanners"]
    check("SelectScanners responds 0 with one scanner", code == 0 and len(scanners) == 1, (code, results))
    sid = scanners[0][0]
    info = scanners[0][1]
    check("scanner info names the SANE backend", info.get("backend") == "test", info)
    check("scanner info carries SANE's device type", info.get("type") == "virtual device", info)

    # ---- permissions
    check("OpenScanner on a scanner that was never granted is NotAllowed", (c.error("OpenScanner", ("deadbeef", {}), "(sa{sv})") or "").endswith("Error.NotAllowed"))
    bad_token = c.error("SelectScanners", ("", {"handle_token": v("s", "has space")}), "(sa{sv})")
    check("invalid handle_token is InvalidArgument", (bad_token or "").endswith("Error.InvalidArgument"), bad_token)

    sess = c.open(sid, "s1")
    check("session path follows the portal convention", sess == f"{PATH}/session/{c.me[1:].replace('.', '_')}/s1", sess)

    # ---- exclusive lock
    check("a second open of the same scanner is Busy (lock)", (c.error("OpenScanner", (sid, {}), "(sa{sv})") or "").endswith("Error.Busy"))

    # ---- a different app cannot use our session
    other = Client()
    foreign = "Error.AccessDenied"
    check("another connection cannot read our session's options", (other.error("GetOptions", (sess, {}), "(oa{sv})") or "").endswith(foreign))
    check("another connection cannot start a frame on our session", (other.error("StartFrame", (sess, {}), "(oa{sv})") or "").endswith(foreign))
    check("another connection cannot Close our session", (other.error("Close", (), "()", iface="org.freedesktop.portal.Session", path=sess) or "").endswith(foreign))

    # ---- options
    opts = c.opts(sess)
    names = [o["name"] for o in opts]
    check("option list has the common options", all(n in names for n in ("mode", "resolution", "tl-x")), names[:8])
    check("array-valued and filename-like options are not exposed", not any("gamma" in n or "file" in n for n in names))
    e = c.error("SetOptions", (sess, [("no-such-option", v("i", 1))], {}), "(oa(sv)a{sv})")
    check("unknown option is InvalidArgument", (e or "").endswith("Error.InvalidArgument"), e)
    e = c.error("SetOptions", (sess, [("mode", v("i", 3))], {}), "(oa(sv)a{sv})")
    check("wrong value type is InvalidArgument", (e or "").endswith("Error.InvalidArgument"), e)
    e = c.error("SetOptions", (sess, [("mode", v("s", "Nonsense"))], {}), "(oa(sv)a{sv})")
    check("value outside the constraint is rejected", e is not None, e)
    applied, flags = c.set(sess, [("mode", v("s", "Color")), ("resolution", v("d", 20.0)), ("test-picture", v("s", "Color pattern"))])
    check("order is kept: mode then resolution applied", applied.get("resolution") == 20.0 and applied.get("mode") == "Color", applied)

    # ---- automatic values: a vardict {auto: true}, only for options with the automatic capability
    auto = v("a{sv}", {"auto": v("b", True)})
    auto_opts = [o["name"] for o in opts if "automatic" in o["capabilities"]]
    check("the test backend has an option with an automatic value", bool(auto_opts), [o["capabilities"] for o in opts][:5])
    if auto_opts:
        # The test backend leaves its extra options inactive until this is on.
        c.set(sess, [("enable-test-options", v("b", True))])
        applied, flags = c.set(sess, [(auto_opts[0], auto)])
        check("an option with the automatic capability can be set to automatic", auto_opts[0] in applied, applied)
    e = c.error("SetOptions", (sess, [("mode", auto)], {}), "(oa(sv)a{sv})")
    check("automatic on an option without the capability is InvalidArgument", (e or "").endswith("Error.InvalidArgument"), e)
    e = c.error("SetOptions", (sess, [("mode", v("a{sv}", {}))], {}), "(oa(sv)a{sv})")
    check("a vardict value without auto=true is InvalidArgument", (e or "").endswith("Error.InvalidArgument"), e)
    e = c.error("SetOptions", (sess, [("mode", v("a{sv}", {"auto": v("b", False)}))], {}), "(oa(sv)a{sv})")
    check("auto=false is InvalidArgument", (e or "").endswith("Error.InvalidArgument"), e)
    c.set(sess, [("mode", v("s", "Color"))])

    # ---- the estimate before a frame matches what the frame then reports
    (est,) = c.call("GetFrameParameters", (sess, {}), "(oa{sv})")
    n0 = len(c.events)
    fd = c.start_frame(sess)
    i, ev = c.wait_state(sess, ("finished", "error", "cancelled"), n0)
    with os.fdopen(fd, "rb") as f:
        data = f.read()
    states = [e[1] for e in c.events[n0:] if e[0] == sess]
    check("normal frame: started then finished", states[:2] == ["started", "finished"], states)
    d = c.events[n0][2]
    check("started carries layout; data length matches", len(data) == d["bytes_per_line"] * d["lines"], (len(data), d))
    check("GetFrameParameters estimate equals the started values", est == {k: x for k, x in d.items() if k != "frame"}, (est, d))
    mine = [e for e in c.events[n0:] if e[0] == sess]
    serial0 = d.get("frame")
    check("every event of a frame carries the frame's number", isinstance(serial0, int) and all(e[2].get("frame") == serial0 for e in mine[:2]), mine)
    check("a frame has exactly one terminal event", len([e for e in mine if e[1] != "started"]) == 1, mine)

    # ---- second StartFrame while one is running
    c.set(sess, [("resolution", v("d", 300.0)), ("read-delay", v("b", True)), ("read-delay-duration", v("i", 200000))]) if "read-delay" in names else None
    n1 = len(c.events)
    fd1 = c.start_frame(sess)
    c.wait_state(sess, ("started",), n1)
    check("the next frame has a higher number", c.events[n1][2].get("frame", 0) > serial0, (c.events[n1][2], serial0))
    e = c.error("StartFrame", (sess, {}), "(oa{sv})")
    check("StartFrame during a frame is Busy", (e or "").endswith("Error.Busy"), e)
    e = c.error("SetOptions", (sess, [("resolution", v("d", 50.0))], {}), "(oa(sv)a{sv})")
    check("SetOptions during a frame is Busy", (e or "").endswith("Error.Busy"), e)
    e = c.error("GetFrameParameters", (sess, {}), "(oa{sv})")
    check("GetFrameParameters during a frame is Busy", (e or "").endswith("Error.Busy"), e)

    # ---- Cancel()
    c.call("Cancel", (sess,), "(o)")
    i, ev = c.wait_state(sess, ("cancelled", "finished", "error"), n1)
    check("Cancel() ends the frame with 'cancelled'", ev and ev[1] == "cancelled", ev)
    os.close(fd1)
    # lock is free for a new frame
    n2 = len(c.events)
    fd2 = c.start_frame(sess)
    c.wait_state(sess, ("started",), n2)
    # ---- closing the pipe early cancels
    os.close(fd2)
    i, ev = c.wait_state(sess, ("cancelled", "finished", "error"), n2)
    check("closing the pipe early ends the frame with 'cancelled'", ev and ev[1] == "cancelled", ev)

    # ---- Cancel() while the app is not reading: the pipe fills and the writer must not hang
    c.set(sess, [("resolution", v("d", 600.0)), ("read-delay", v("b", False))])
    n = len(c.events)
    fdx = c.start_frame(sess)
    c.wait_state(sess, ("started",), n)
    time.sleep(0.5)  # the 64 KB pipe is full by now and nobody reads
    t0 = time.time()
    c.call("Cancel", (sess,), "(o)")
    i, ev = c.wait_state(sess, ("cancelled", "finished", "error"), n, timeout=8)
    check("Cancel() with a full, unread pipe still ends the frame", ev and ev[1] == "cancelled", ev)
    check("... and promptly", time.time() - t0 < 2, time.time() - t0)
    os.close(fdx)
    # and the portal still answers other work afterwards
    check("portal still serves requests after that", c.opts(sess) != [])

    # ---- client vanishes: lock must be released
    c.call("Close", (), "()", iface="org.freedesktop.portal.Session", path=sess)
    sess = c.open(sid, "s2")
    check("after Close the scanner can be opened again", bool(sess))
    ghost = Client()
    ghost.select()
    gsess_err = ghost.error("OpenScanner", (sid, {}), "(sa{sv})")
    check("scanner is held while c has it open (Busy)", (gsess_err or "").endswith("Error.Busy"), gsess_err)
    c.call("Close", (), "()", iface="org.freedesktop.portal.Session", path=sess)
    gs = ghost.open(sid, "g1")
    ghost.bus.close_sync(None)
    time.sleep(1.0)
    again = c.open(sid, "s3")
    check("scanner is released when its app leaves the bus", bool(again))
    sess = again

    # ---- hand scanner: unknown height
    # The resolution is set here because a session does not have to inherit the previous one's
    # options: the prototype keeps them on the device, the frontend in C starts a helper per session.
    c.set(sess, [("resolution", v("d", 50.0)), ("hand-scanner", v("b", True))])
    n3 = len(c.events)
    fd = c.start_frame(sess)
    c.wait_state(sess, ("started",), n3)
    d = c.events[n3][2]
    with os.fdopen(fd, "rb") as f:
        f.read()
    check("hand scanner reports lines = -1", d["lines"] == -1, d)
    c.wait_state(sess, ("finished",), n3)
    c.set(sess, [("hand-scanner", v("b", False))])

    # ---- feeder runs out of documents
    c.set(sess, [("source", v("s", "Automatic Document Feeder")), ("resolution", v("d", 10.0))])
    seen = None
    for k in range(13):
        n = len(c.events)
        fd = c.start_frame(sess)
        with os.fdopen(fd, "rb") as f:
            f.read()
        i, ev = c.wait_state(sess, ("finished", "no-documents", "error", "cancelled"), n)
        if ev and ev[1] != "finished":
            seen = (k, ev)
            break
    check("empty feeder reports 'no-documents' (after 10 scans)", seen is not None and seen[1][1] == "no-documents", seen)

    c.call("Close", (), "()", iface="org.freedesktop.portal.Session", path=sess)
    print()
    print("FAILED: " + ", ".join(failures) if failures else "all scenarios passed")
    return 1 if failures else 0


sys.exit(main())
