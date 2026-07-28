# Spool Linux shell

The Linux V1 shell is a StatusNotifierItem/AppIndicator client of the local IPC
V1 contract. It is packaged separately from the systemd agent and never gains
access to the agent database or device credential.

Desktop environments without StatusNotifier support run Spool headlessly and
use `spoolctl` or the loopback UI. The package is advertised as Preview until
GNOME, KDE and an AppIndicator fallback pass login-startup and upgrade gates.

