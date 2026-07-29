# Windows service status: Disabled

The V1 `spool-agent.exe` is a console process. It does not implement the Windows
Service Control Manager lifecycle, so registering it with `sc.exe create` would
produce a service that times out with error 1053. This repository deliberately
does not ship a fake service template or claim background-service support.

The current notification-area executable is icon-only: it does not read agent
status or expose operational actions. Both the Windows service and shell remain
**Disabled** until a real SCM host, restricted service identity, named-pipe ACL,
recovery policy, install/uninstall lifecycle, and Windows integration tests are
implemented.

Developers may run `spool-agent.exe` interactively for evaluation. That is not a
production persistence mechanism.
