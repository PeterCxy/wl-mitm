wl-mitm
---

`wl-mitm` is a filtering man-in-the-middle proxy for Wayland compositors.
Through a toml config file, it allows you to selectively enable only a necessary subset of Wayland protocols
for apps to function.

Wayland's core protocols are generally safe to expose to any program, but there are extensions that could
potentially be abused. For example, [wlr-screencopy-unstable-v1](https://wayland.app/protocols/wlr-screencopy-unstable-v1).
A sandboxed program may be able to make use of these extensions to "escape", at least in the sense of accessing
screen content, clipboard, etc.

Moreover, access control to sensitive protocols implemented in different compositors may be wildly different.
Although something like [security-context-v1](https://wayland.app/protocols/security-context-v1) exists, what ends up
being exposed is still determined by the compositor and this behavior is usually not programmable by the end user.

`wl-mitm` solves this by _proxying_ the Wayland socket and exposing a very flexible configuration format to
you, the user. You can elect to allow any subset of Wayland protocols you would like an application to have access
to, or even filter specific _requests_ provided that clients can handle a missed message well. `ask_cmd` and
`notify_cmd` combines this filtering capability with arbitrary extensibility through external programs. For example,
an `ask_cmd` may show a prompt to the user whether to allow a certain Wayland request to proceed. A `notify_cmd`
may send a desktop notification when an application performs a senstive action through a Wayland request.
This also makes it potentially a very useful debugging tool.

`wl-mitm` is intended to be used _on top of_ some other sandboxing system that, at the very least, limits
a program's access to the rest of the filesystem. Otherwise, a malicious application can simply access the original,
unadulterated Wayland socket directly under `$XDG_RUNTIME_DIR`.

Building
---

`wl-mitm` requires a Rust compiler supporting Rust 2024.

`wl-mitm` relies on generated code and does not use `build.rs` or `proc_macro`s due to performance issues with
`rust-analyzer`. Instead, run `./generate.sh` to generate Rust parser code based on Wayland protocol XMLs located
under `proto/`.

After running `./generate.sh`, simply run `cargo build --release` to produce a release binary.

Usage
---

Run `wl-mitm` with

```
wl-mitm <path/to/configuration/file>
```

Path to the configuration file defaults to `./config.toml`.

This repo contains an example configuration at `config.toml` that allows a few base Wayland protocols for standard
desktop apps to function. It also demonstrates the use of `ask_cmd` and `notify_cmd` by defining filters on clipboard-related
requests. Detailed explanation of the configuration format is also contained in the example.

To launch a program under `wl-mitm`, set its `WAYLAND_DISPLAY` env variable to whatever `listen` is under `[socket]` in `config.toml`.
Note that you may want to use another container and pass _only_ the `wl-mitm`'d socket through for proper isolation.

A Word on Filtering
---

`wl-mitm` gives you a ton of flexibility on filtering. In particular, you are allowed to filter _any_ request on _any_ object
in `config.toml`.

However, please keep in mind that while most requests can be filtered, Wayland (and most of its protocols) is not designed to handle
this type of filtering. While a lot of requests can be filtered without consequences, there are a significant number of them that
will result in irrecoverable de-sync between the client and the server. For example, _any_ message that creates a new object ID
(of the type `new_id`) _will_ result in undefined behavior if filtered.

`wl-mitm` does provide the ability to _reject_ a request with an explicit error. Unfortunately, errors in Wayland are usually
fatal, and clients are not designed to recover from them.

The most reliable (and the only protocol-compliant) way to prevent a client from using certain features is to block the
entire global object where that feature resides. These are usually "manager" objects that live in the global registry,
and can be blocked in `allowed_globals` inside `config.toml`. Each extension protocol has to expose _some_ manager object
in order for clients to access their functionalities, and thus blocking them there will prevent clients from even knowing
their existence. However, globals can't be selectively blocked using `ask_cmd`, because clients usually bind to them as soon
as they start.

Since `notify_cmd` never blocks a request, it is safe to use on _any_ request filter.

XWayland
---

`wl-mitm` is explicitly _not_ a sub-compositor. This also means that it can't support XWayland natively like some
subcompositors do, such as [sommelier](https://chromium.googlesource.com/chromiumos/platform2/+/refs/heads/main/vm_tools/sommelier/) from
ChromeOS. The decision to _not_ implement it as a sub-compositor is made to ensure maximum compatibility with Wayland
protocols and compositors, and to introduce as little additional quirks as possible over the host compositor.

If you would like to use XWayland over a socket filtered by `wl-mitm`, here are a few options:

1. Use Sommelier's X mode only;
2. [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite) is a compositor-agnostic XWayland implementation in Rust;
3. [gamescope](https://github.com/ValveSoftware/gamescope) is an XWayland-only subcompositor specifically intended for games.

Supported Protocols
---

`wl-mitm` aims to include support for all known, non-deprecated Wayland protocols. This is currently achieved by
pulling in all XMLs from the [wayland-explorer](https://wayland.app) project.

The `update-proto.sh` script is responsible for updating the list of XMLs used to generate Wayland parsers.
