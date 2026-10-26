#!/usr/bin/env python3
"""Test double for org.freedesktop.impl.portal.Scanner, the part of the portal that draws the dialog.

It shows nothing and answers at once: it grants the first scanner it is offered, or every scanner
when the application asked for `multiple`. That lets the scenario checks run against the real
frontend without someone pressing Return. The real dialog is tested on its own.

    impl_auto_grant.py BUS_NAME IMPL_INTERFACE_XML
"""
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

PATH = "/org/freedesktop/portal/desktop"


def main():
    name, xml_path = sys.argv[1], sys.argv[2]
    with open(xml_path) as f:
        node = Gio.DBusNodeInfo.new_for_xml(f.read())
    iface = node.interfaces[0]

    def on_call(_conn, _sender, _path, _iface, method, params, invocation):
        if method != "SelectScanners":
            invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method)
            return
        _handle, _parent, app_id, scanners, options = params.unpack()
        ids = [s[0] for s in scanners]
        chosen = ids if options.get("multiple") else ids[:1]
        print(f"SelectScanners for {app_id!r}: offered {ids}, granting {chosen}", flush=True)
        if not chosen:
            invocation.return_value(GLib.Variant("(ua{sv})", (1, {})))
            return
        invocation.return_value(GLib.Variant("(ua{sv})", (0, {"scanners": GLib.Variant("as", chosen)})))

    def on_bus(conn, _name):
        conn.register_object(PATH, iface, on_call, None, None)

    Gio.bus_own_name(Gio.BusType.SESSION, name, Gio.BusNameOwnerFlags.NONE, on_bus, None, None)
    GLib.MainLoop().run()


main()
