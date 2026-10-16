//! Test double for the proposed scanner portal. See the crate docs.
//!
//! Usage: nv-portal-proto [--name BUS_NAME] [--select SUBSTRING] [--network]

use futures_util::StreamExt;
use nv_portal_proto::service::{self, Policy, Scanner, Shared};
use nv_portal_proto::{DEFAULT_BUS_NAME, PORTAL_PATH, sane_thread};
use std::sync::mpsc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut name = DEFAULT_BUS_NAME.to_owned();
    let mut policy = Policy { select: None, network: false };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => name = args.next().ok_or("--name needs a value")?,
            "--select" => policy.select = Some(args.next().ok_or("--select needs a value")?),
            "--network" => policy.network = true,
            _ => return Err(format!("unknown argument {a}").into()),
        }
    }

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (evt_tx, evt_rx) = async_channel::unbounded();
    sane_thread::spawn(cmd_rx, evt_tx);
    let shared = Shared::new(cmd_tx, policy);

    zbus::block_on(async move {
        let conn = zbus::connection::Builder::session()?.name(name.as_str())?.serve_at(PORTAL_PATH, Scanner(shared.clone()))?.build().await?;
        eprintln!("nv-portal-proto: serving {PORTAL_PATH} as {name}");

        conn.executor().spawn(service::forward_events(conn.clone(), evt_rx), "frame-events").detach();

        // Release a scanner when the app that opened it leaves the bus.
        let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
        let mut owners = dbus.receive_name_owner_changed().await?;
        while let Some(sig) = owners.next().await {
            let args = sig.args()?;
            if args.new_owner().is_none()
                && let Some(old) = args.old_owner().as_ref()
            {
                shared.close_all_for(&conn, old.as_str()).await;
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
