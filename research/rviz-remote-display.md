# RViz2 from a robot devcontainer to tranquility

Research date: 2026-10-08. Feasible in principle; no robot, remote display, GPU or ROS graph was contacted or tested. Existing evidence describes Arch tranquility, ARM64 CoreOS Blastoise and ARM64 Ubuntu 20.04 Jetson. Current ROS distro, GPU access, display services and container network modes still need verification.

## Recommendation

For regular visualization, run a matching ROS/RViz environment on tranquility and subscribe to the robot's ROS topics. The laptop renders locally; no robot desktop server is required. A Linux development container on tranquility can provide the matching ROS distro, messages and RViz plugins without changing its host ROS installation. Start with the existing robot LAN connection; consider a scoped DDS relay only if direct DDS traffic cannot traverse the mesh.

When the application must run inside the existing robot devcontainer, test a private SSH-tunneled TurboVNC display, with VirtualGL for compatible remote GPUs. This carries rendered pixels and input while keeping the existing ROS environment on the robot. An initial software-rendered VNC test can establish compatibility without exposing GPU devices. This is a separate, opt-in graphics session, rather than a new setting on every terminal.

| Approach | Where rendering runs | Assessment |
|---|---|---|
| Local RViz + remote ROS topics | tranquility GPU | Preferred for normal RViz use; requires ROS data connectivity and matching messages/plugins. |
| TurboVNC + VirtualGL | robot GPU, pixels sent to tranquility | Best candidate for the exact remote application; ARM64 packages exist, but Jetson/ODROID driver compatibility needs testing. |
| TurboVNC + Mesa software rendering | robot CPU | Useful initial test/fallback; measure load before using with a running robot. |
| SSH X11 forwarding | indirect GLX path to laptop X server | Small GUI apps may work; RViz OpenGL is a separate, often limiting requirement. |
| Waypipe | remote app, graphics buffers sent to laptop | Potential future adapter; native Wayland alone does not satisfy RViz's current Linux X11 renderer. |

## What needs passthrough

RViz is a Qt/Ogre/OpenGL application, not terminal output. Both inspected Humble and rolling renderers call `XOpenDisplay` and create a GLX context; Humble explicitly requires at least OpenGL 2.1.[1] A working terminal, clipboard and terminal colors therefore do not establish GUI support. If tranquility uses Wayland, Xwayland can display X11 apps, but that does not guarantee usable indirect GLX over SSH. The laptop's current display server has not been verified.

OpenSSH `-X` sets a connection-specific `DISPLAY` and remote Xauthority credentials; `-Y` enables trusted X11 forwarding.[2] `-Y` enlarges access to the local display and does not create a missing OpenGL capability. Do not use `xhost +` or expose an unauthenticated X TCP listener as a workaround.

An SSH-assigned display such as `localhost:10.0` points to the robot host's forwarding listener. In a bridge-networked container, `localhost` refers to the container, so passing that string to `docker exec` is insufficient. Linux host-network containers share the host network namespace; otherwise a deliberately scoped proxy is needed.[3] The application's container user also needs access to the particular connection's Xauthority credentials. Existing containers cannot acquire new bind mounts through `docker exec`; use a per-session temporary credential file with cleanup, or a separately provisioned graphical development profile. Do not recreate a working ROS container merely to change its network mode.

VirtualGL intercepts OpenGL calls, renders remotely and sends completed images. Its GLX backend needs an appropriate X server; its EGL backend can render without a 3D X server when the driver supports `EGL_EXT_platform_device`.[4] TurboVNC provides an optimized X display/image transport. Current upstream releases provide ARM64 Linux packages for both projects.[5] This proves availability, not compatibility with the specific Jetson JetPack/NVIDIA or ODROID GPU stack. NVIDIA container runtime/driver libraries and EGL support need individual checking; CPU rendering remains a possible fallback.

Waypipe proxies Wayland clients over SSH and can transport graphics buffers.[6] Setting `QT_QPA_PLATFORM=wayland` alone does not remove RViz's X11/GLX renderer. The current manual offers `--xwls`, requiring `xwayland-satellite`; older boat distributions may lack it. An Xwayland compatibility compositor and container-visible private socket would need testing. Do not promise direct native Wayland RViz support based only on Qt support.

## ROS data across the mesh

Local RViz should use the same ROS distro, matching custom message definitions and required display plugins as the robot. ROS domain, RMW interoperability, topic QoS, `/tf`, `/tf_static`, timestamps and assets such as URDF meshes all matter. RViz tools can publish goals/initial poses; a display is not automatically read-only.

Default DDS discovery commonly depends on multicast. Static peers or Fast DDS Discovery Server can solve particular discovery problems, but discovery alone does not relay user samples through NAT or an SSH-only path.[7] `ROS_STATIC_PEERS` is not available on every older distro/RMW, and ROS documents that improved-discovery variables are unsupported by `rmw_zenoh`; inspect the actual stack before choosing settings. Docker bridge addressing introduces another boundary.

For routed/SSH-only connectivity, an explicit DDS Router pair is a candidate: one local participant on each ROS LAN, connected by TCP through a private SSH forward, with a topic allowlist.[8] Validate actual locator/port configuration and data flow through the tunnel rather than assuming forwarding one discovery port works. eProsima documents that TCP client/server direction is independent of bidirectional DDS communication. Preserve the user's no-reverse-SSH policy: an established graphics or DDS connection can carry replies without granting robot SSH access to mesh hosts. Topic authorization, GUI access and SSH authorization are distinct controls.

## Fit with CX

Current CX assembles saved SSH/ProxyJump paths in `src/store.rs:223`, with a shared control path at `src/store.rs:270`; it does not explicitly request `-X`/`-Y`, although a user's SSH config may. `src/sessions.rs:1460` attaches through the remote helper/tmux. `src/containers.rs:770` forwards terminal/theme variables and configured Dev Containers remote environment, but has no automatic display-cookie/socket transport.

Use a separate client-owned graphics connection. OpenSSH multiplexed X11 forwarding uses the master connection's display; disable multiplexing for a bounded GUI probe (`-S none`) rather than mutating CX's shared connection.[2] Persistent tmux sessions can retain stale `DISPLAY`/`XAUTHORITY` after detach or viewer changes. Avoid global tmux environment updates: tie display credentials, forward, process and cleanup to the graphical viewer. Reattaching a shell must not silently select another viewer's display. A future adapter should probe display/OpenGL support, offer an explicit “Open graphical app” action and report measured capability separately from ordinary terminal availability.

## Bounded validation plan

1. Read only: identify the active `roboboat_dev` container by full ID; inspect only architecture, ROS distro, RMW, network mode, configured GPU devices/runtime and relevant display binaries. On tranquility identify X11/Wayland/Xwayland availability. Do not dump credentials or the full environment.
2. On tranquility's graphical desktop, create one short-lived separate SSH connection to the existing robot target: `ssh -S none -X roboboat@192.168.0.153` (Blastoise's saved IP is `.147`; verify it before use). Tranquility can use its existing direct LAN route; another viewer can use the saved ProxyJump route. Keep host-key checking enabled. In that connection run `timeout 15s xdpyinfo` and `timeout 15s glxinfo -B`, if present. Simple X11 success and OpenGL success are separate evidence. Consider trusted `-Y` only as an explicit comparison with a known-trusted remote process, not the default fix.
3. For an existing host-networked devcontainer, prove one short-lived process can use that exact SSH display and connection-specific auth file as the intended non-root user. For bridge networking, stop at the namespace barrier and design a scoped proxy/profile rather than changing the robot's live network. Run a 30-second RViz launch in the same graphics connection, never a shared tmux shell. Close only the test process and its temporary auth material.
4. If GLX fails, provision a separate opt-in VNC test environment after reviewing the required changes. Bind VNC to robot loopback; forward one private local port through SSH, for example `ssh -S none -N -L 127.0.0.1:15901:127.0.0.1:5901 roboboat@192.168.0.153` only after verifying `5901` belongs to the test display. Test software rendering first, then VirtualGL if compatible GPU access is already available. Record RViz launch, OpenGL renderer, camera interaction, representative PointCloud2/Image/TF performance and robot CPU/GPU load. Do not expose VNC on the robot LAN. A loopback listener inside a bridge-networked container is not the host loopback listener: use a scoped, authenticated host-local proxy or SSH stream into that namespace before using this forwarding example; do not switch the existing container to host networking.
5. Independently test local RViz on tranquility with a matching ROS environment: bounded topic discovery, one sample of required visualization topics and correct TF display/QoS. Use a harmless test publisher when available. If mesh DDS fails, test a filtered TCP DDS relay in a disposable test domain before applying it to live robot topics; do not alter DD-WRT, WARP, default gateways or existing robot containers.
6. Verify cleanup, reconnect behavior and that robot SSH access back to mesh devices remains unchanged. Physical graphics quality, ROS functionality and GPU performance remain acceptance tests requiring tranquility and the boat.

## Primary sources

1. RViz renderer source: [Humble](https://github.com/ros2/rviz/blob/humble/rviz_rendering/src/rviz_rendering/render_system.cpp), [rolling](https://github.com/ros2/rviz/blob/rolling/rviz_rendering/src/rviz_rendering/render_system.cpp).
2. OpenSSH [ssh(1)](https://man.openbsd.org/ssh.1): X11 forwarding, `-X`/`-Y`, authorization and multiplexing.
3. Docker [host networking](https://docs.docker.com/engine/network/drivers/host/) and [exec environment/process behavior](https://docs.docker.com/reference/cli/docker/container/exec/).
4. VirtualGL [User's Guide](https://virtualgl.org/Documentation/Documentation), especially backend design, sections 6.2/6.3 and X proxy operation in section 9; [upstream guide source](https://github.com/VirtualGL/virtualgl/blob/main/doc/index.html).
5. Upstream [VirtualGL releases](https://github.com/VirtualGL/virtualgl/releases), [TurboVNC releases](https://github.com/TurboVNC/turbovnc/releases).
6. [waypipe(1)](https://man.archlinux.org/man/waypipe.1.en), including `--xwls` and graphics constraints.
7. ROS [improved discovery source](https://github.com/ros2/ros2_documentation/blob/rolling/source/Developer-Tools/Introspection-and-analysis/Improved-Dynamic-Discovery.rst), [Discovery Server tutorial source](https://github.com/ros2/ros2_documentation/blob/rolling/source/Developer-Tools/Introspection-and-analysis/Discovery-Server/Discovery-Server.rst). These are current documentation, not proof that the robots' older ROS installations expose every setting.
8. eProsima [DDS Router WAN-over-TCP tutorial](https://github.com/eProsima/DDS-Router/blob/main/docs/rst/use_cases/wan_tcp.rst): routing, topic filters and connection direction.
