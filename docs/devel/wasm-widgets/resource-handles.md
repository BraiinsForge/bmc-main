# WASM Widget Resource Handles

Each SDK call in the table below starts something in the background and returns a handle to it. The handle is the only
one for that resource: it cannot be copied, and dropping it ends the resource. A fetch or an image decode returns a
plain id instead, and ends on its own.

| Call                                                         | Handle             | Ends it        |
| ------------------------------------------------------------ | ------------------ | -------------- |
| `timer::start`                                               | `Timer`            | `stop()`       |
| `socket::tls_connect`, `tls_connect_insecure`, `tcp_connect` | `Socket`           | `close()`      |
| `ws::ws_connect`, `ws!`                                      | `Ws`               | `close()`      |
| `mdns::mdns_browse`                                          | `MdnsBrowse`       | `stop()`       |
| `mdns::mdns_register`                                        | `MdnsRegistration` | `unregister()` |
| `ssdp::ssdp_search`                                          | `SsdpSearch`       | `stop()`       |
| `udp_broadcast::udp_broadcast`                               | `UdpBroadcast`     | `stop()`       |
| `http_listener::http_listen`                                 | `HttpListener`     | `close()`      |
| `register_poll`                                              | `PollHandle`       | `stop()`       |

Each call except `register_poll` returns `None` when the host refuses the resource. The host logs why when a runtime
limit refuses it, a listener's port cannot be bound, or the mDNS daemon cannot register a service; input it cannot use,
such as an empty list of service types, is refused without a log. A hermetic capture refuses `http_listen` and
`mdns_register`; the other network calls get a handle that receives only the events in the capture's fixture, and none
when the fixture has none. A native build, such as a widget's unit tests, has no host and none of these calls.

## Keep The Handle

A resource lives as long as its handle, unless the other end ends it first, so keep the handle for as long as the widget
wants the resource, typically in a `thread_local!` slot:

```rust
use std::cell::RefCell;

use bmc_wasm_sdk::mdns::{self, MdnsBrowse, MdnsEvent};
use bmc_wasm_sdk::*;

thread_local! {
    static BROWSE: RefCell<Option<MdnsBrowse>> = const { RefCell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let browse = mdns::mdns_browse(&["_http._tcp"], on_mdns_event);
    if browse.is_none() {
        log_warn!("mDNS browse rejected by host runtime limits");
    }
    BROWSE.set(browse);
}

fn on_mdns_event(_browse: &MdnsBrowse, event: &MdnsEvent<'_>) {
    if let MdnsEvent::Found(json) = event {
        log_info!("found: {json}");
    }
}
```

Checking the result with `mdns_browse(...).is_none()` and keeping nothing compiles without a warning, and ends the
browse as soon as `init` returns. So does `let _ = mdns_browse(...)`, which is what the compiler suggests for an unused
result. The SDK offers no way to detach a handle and leave its resource running, so short of leaking the handle
(`std::mem::forget`, `ManuallyDrop`, etc.), every background resource a widget starts stays one it can stop.

## End A Resource

Call the method in the table, or drop the handle; both do the same. The method consumes the handle, so using it
afterwards does not compile. An event for the resource already due in the same delivery pass is dropped, so its callback
never runs after the resource ends.

## Callbacks Borrow The Handle

A callback receives a borrowed handle, `&Socket` or `&Ws` for example, so it can use the resource but not end it. To end
the resource from its own callback, take the stored handle out of its slot:

```rust
fn on_socket_event(socket: &Socket, event: &SocketEvent<'_>) {
    match event {
        SocketEvent::Connected => socket.write(b"hello"),
        SocketEvent::Data(b"bye") => {
            if let Some(socket) = SOCKET.take() {
                socket.close();
            }
        }
        SocketEvent::Data(_) => {}
        SocketEvent::Closed(_) => SOCKET.set(None),
    }
}
```

The borrowed handle stays usable after that, but does nothing: a write or a send on a closed connection is dropped.

When the peer closes a socket or a WebSocket, the callback gets `SocketEvent::Closed` or `WsEvent::Close`, and the
stored handle reaches nothing from then on. Clear or replace it there, so a stored `Some` keeps meaning connected.

Several handles of one kind can share a callback. To find the one an event belongs to, compare the argument with the
stored handles.

## Poll Handles

`PollHandle::stop()` ends a poll, and so does dropping the handle. Either one cancels the poll's live fetch, a reply
already on its way is discarded, and the reply handler never runs again. `set_enabled(false)` only pauses a poll.

A poll's `build` function runs while the SDK holds its poll registry, so calling any poll handle method but `index()`
from inside `build` panics, and so does ending a poll there. End a poll from a reply handler or a lifecycle hook
instead.

`PollHandle::index` numbers polls in registration order, and an ended poll's index is never given to another poll, so
widget state keyed by it stays unambiguous. Each ended poll keeps an empty slot for the widget's lifetime, so a widget
that registers new polls over and over grows without bound.
