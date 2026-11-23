# WASM Widget Struct

A widget is a struct that implements `Widget`, with `#[widget]` on the impl block.

- The SDK owns the one instance: `init` builds it, every other hook runs as a method on it, and `unload` takes it by
  value as its last call.
- `#[widget]` exports `init`, `render`, `unload` and only the optional hooks the impl writes: a widget that leaves out
  `on_touch` gets no touches.
- A hook the impl leaves out cannot be exported by hand either: `#[widget]` holds its name, so a hand-written `on_touch`
  fails to compile for wasm32 on the duplicate symbol rather than run outside the instance.
- One `#[widget]` per crate: a second one fails to compile for wasm32 on the duplicate `init`.

```rust
use bmc_wasm_sdk::*;

struct RandomFact {
    fact: Option<String>,
    _poll: Poll,
}

impl RandomFact {
    fn build_request(&mut self, _poll: &Poll) -> Option<FetchSpec> {
        Some(FetchSpec::get(FACTS_API_URL))
    }

    fn on_fact_data(&mut self, _poll: &Poll, response: &FetchResponse) -> Verdict {
        let Some(fact) = response.ok().then(|| response.json().str("/data/text")).flatten() else {
            return Verdict::Retry;
        };
        self.fact = Some(fact);
        request_frame();
        Verdict::Accept
    }
}

#[widget]
impl Widget for RandomFact {
    fn init(me: &Building<Self>) -> Self {
        let poll = Poll::register(me, Self::build_request, Self::on_fact_data, PollConfig::default());
        Self { fact: None, _poll: poll }
    }

    fn render(&mut self, _delta_ms: u32) {
        let fact = self.fact.as_deref().unwrap_or("Loading...");
        // …
    }

    fn on_system_update(&mut self) {
        request_frame();
    }
}
```

What runs when, for this widget:

1. The host calls `init`. The poll registers through `me`, but its first `build_request` waits: the instance does not
   exist until `init` returns.
2. `init` returns, the SDK stores the instance, and before the host gets control back `build_request` runs on it and the
   request goes out.
3. In a later host call the reply arrives, and `on_fact_data` runs on the instance.
4. A `Verdict::Retry` returned there only queues the next request. Once `on_fact_data` has returned, `build_request`
   runs and the request goes to the host, to be sent after `retry_ms`; only then does the export end and the host get
   control back. An `invalidate()` from a hook such as `on_params_update` runs the same way: the hook returns,
   `build_request` runs, the export ends.
5. `unload` drops the instance, and with it `_poll`, which ends the poll.

## Callbacks Are Methods

Every SDK call that takes a callback first takes what the callback runs on, its receiver; the callback is a method of
the receiver's type.

| Free function (obsolete)      | Receiver first                                               |
| ----------------------------- | ------------------------------------------------------------ |
| `timer::start`                | `Timer::start`                                               |
| `register_poll`               | `Poll::register`                                             |
| `request.send`, `send_after`  | `request.send_on`, `send_after_on`                           |
| `slot.set_fit`, `set_fit_ref` | `slot.set_fit_on`, `set_fit_ref_on`                          |
| `set_bitmap_fit`              | `set_bitmap_fit_on`                                          |
| `ws_connect`, `ws!`           | `Ws::connect`                                                |
| `tls_connect`, …              | `Socket::tls_connect`, `tls_connect_insecure`, `tcp_connect` |
| `mdns_browse`                 | `MdnsBrowse::start`                                          |
| `ssdp_search`                 | `SsdpSearch::start`                                          |
| `udp_broadcast`               | `UdpBroadcast::start`                                        |
| `http_listen`                 | `HttpListener::listen`                                       |

The receiver is whatever the calling code has at hand:

| Receiver    | Passed from                                                | The callback runs on        |
| ----------- | ---------------------------------------------------------- | --------------------------- |
| `self`      | a method of the widget                                     | the widget                  |
| `me`        | `init`, or the build of a part's `Part::new_cyclic`        | the value, once it is built |
| `&part`     | the widget, which holds the `Part<T>`                      | the part's value            |
| `&self.me`  | a part's value, through a `PartHandle` it keeps            | the part's value            |
| `self.me()` | a method of the widget, to code that runs on a field of it | the widget                  |

What a registration returns depends on its receiver:

| Registration                                                     | Through `self`, `self.me()`, `me` or `&part` | Through a `PartHandle`      |
| ---------------------------------------------------------------- | -------------------------------------------- | --------------------------- |
| one the host may refuse, whose free function returns `Option<X>` | `Result<X, RegisterError>`                   | `Result<X, HandleError>`    |
| `set_fit_ref_on`, whose free function returns `Result<X, E>`     | `Result<X, RegisterError<E>>`                | `Result<X, HandleError<E>>` |
| `Poll::register` or `Notify::new`, never refused                 | the handle `H` itself                        | `Result<H, PartError>`      |

- The free `fetch` and `fetch_after` have no row: send a `FetchRequest` with `send_on` instead.
- The method takes `&mut self`, then the arguments the callback has always had. A poll's methods take a `&Poll` in place
  of the `&PollHandle`, and its reply method returns a `Verdict`. A decode's method takes a `DecodeOutcome` in place of
  the `Option<BitmapId>`. A method of a type other than the receiver's does not compile, nor does a part passing `self`:
  `` `Session` cannot receive callbacks for `Session`'s methods ``.
- `into_inner` on a `RegisterError` hands back what the free function's error carries, such as the body.
  `HandleError::Part` says the registration never reached the host, `HandleError::Register` that the host refused it.
  `?` turns a `PartError`, or a `RegisterError` from another receiver, into a `HandleError`.
- `PartError` is `#[non_exhaustive]`, so a `match` on it outside the SDK ends with a `_` arm. `HandleError` is not: a
  registration either reached the host or did not.
- For `send_on` and `send_after_on`, `X` is a `Fetch`, not the free `send`'s `FetchRequestId`: it owns the request, and
  dropping it cancels the request.
- A callback runs as one call into the widget, like a hook: on the one instance, with nothing else borrowing it, on the
  fuel budget of the host call that delivers it and under the import rules of the `on_*` hooks (the lifecycle guard
  matrix in the `bmc_wasm_sdk` crate docs).
- The free functions take a plain `fn` for a widget that keeps its state in `thread_local!` cells. They stay only so
  that a widget not yet ported still builds.

## Parts

A sub-object of the widget, a session or a screen say, owns its own timers and polls when it lives in a `Part<T>`.
`with` and `with_mut` lend its value for one call; dropping the part drops the value and the handles it holds.

```rust
use bmc_wasm_sdk::timer::Timer;

struct Session {
    hashrate: Option<f64>,
    sampler: Option<Timer>,
}

impl Session {
    fn new() -> Part<Self> {
        Part::new_cyclic(|me| Self {
            hashrate: None,
            sampler: Timer::start(me, 60_000, Self::sample).ok(),
        })
    }

    fn sample(&mut self, _timer: &Timer, _elapsed_periods: u32) {
        // …
    }
}

type Feed = Part<Session>;

struct Dashboard {
    feed: Option<Feed>,
}
```

- A callback runs as the value's method for as long as the part lives; once it is dropped, its registrations deliver
  nothing.
- The value reaches only its own fields and handles, not the widget. The widget reaches the value through the part, from
  its own methods: `feed.with(|value| value.hashrate)`.
- Name the part with a type alias, as `Feed` above, so fields name the sub-object rather than the box. Still construct
  it on the value's type, `Session::new()`: the alias's `new` is `Part::new`, which takes the value.

### A Part That Registers Its Own Callbacks

A chain such as a fetch whose reply starts a decode needs the value to name its own part after it is built. It keeps the
`PartHandle<T>` that `me.handle()` gives while it is built:

```rust
struct Loader {
    me: PartHandle<Loader>,
    fetch: Option<Fetch>,
}

impl Loader {
    fn new(url: &str) -> Part<Self> {
        let mut loader = Part::new_cyclic(|me| Self { me: me.handle(), fetch: None });
        loader.with_mut(|value| value.fetch(url));
        loader
    }

    fn fetch(&mut self, url: &str) {
        match FetchRequest::get(url).send_on(&self.me, Self::on_fetched) {
            Ok(fetch) => self.fetch = Some(fetch),
            Err(HandleError::Register(_)) => log_warn!("fetch refused, past the widget's limit"),
            Err(HandleError::Part(_)) => {}
        }
    }

    fn on_fetched(&mut self, response: &FetchResponse) {
        let fit = Fit::Within { w: 64, h: 64 };
        let decode = set_bitmap_fit_on(&self.me, "art", response.body(), fit, b"", Self::on_decoded);
        if let Err(HandleError::Register(_)) = decode {
            log_warn!("decode refused, one is already in flight");
        }
    }

    fn on_decoded(&mut self, _job: ImageJobId, _outcome: DecodeOutcome) {}
}
```

- The handle is weak: it never keeps the part alive. It counts as live only once the build has returned: inside the
  build, register through `me`, as anything through the stored handle fails with `PartError::NotLive`. `Loader::new`
  starts its fetch once `new_cyclic` has returned.
- `send_on` returns a `Fetch`, which cancels the request when dropped, so dropping the `Loader` cancels its fetch too.
- A decode's `WhileDormant` says the decode ended while the widget slept, which a free callback never hears. It carries
  no result; its doc says how to recover after `on_wake`.
- While the handle is not live, a form through it fails with `PartError::NotLive` without asking the host, and
  `slot.set_fit_ref_on` drops the body with nothing left to decode it for. When the host refuses, the form returns
  `HandleError::Register`, carrying the `RegisterError` it returns through `self`, `me` or `&part`.
- A callback that drops its own part keeps the value until it returns; a registration it makes after the drop is
  accepted and delivers nothing.
- A part cannot reach the widget while its callback runs. It tells the widget through a `Notify`, and the widget reacts
  once the callback has returned. A sub-object that needs the widget inside its callback cannot be a part: the widget
  owns it as a plain field and runs its callbacks itself, passing `self.me()` to the field for the registrations it
  makes.

### A Part That Tells the Widget

When the widget should react to a part, it hands the part a `Notify` bound to one of its own methods, and the part sends
through it. A part that fetches for this one widget sends what it fetched, and the widget keeps it:

```rust
struct FactSource {
    _poll: Poll,
    on_fact: Notify<String>,
}

impl FactSource {
    fn new(on_fact: Notify<String>) -> Part<Self> {
        Part::new_cyclic(|me| Self {
            _poll: Poll::register(me, Self::build_request, Self::on_reply, PollConfig::default()),
            on_fact,
        })
    }

    fn build_request(&mut self, _poll: &Poll) -> Option<FetchSpec> {
        Some(FetchSpec::get(FACTS_API_URL))
    }

    fn on_reply(&mut self, _poll: &Poll, response: &FetchResponse) -> Verdict {
        let Some(fact) = response.ok().then(|| response.json().str("/data/text")).flatten() else {
            return Verdict::Retry;
        };
        self.on_fact.send(fact);
        Verdict::Accept
    }
}

type Facts = Part<FactSource>;

struct FactCard {
    _source: Facts,
    fact: Option<String>,
}

#[widget]
impl Widget for FactCard {
    fn init(me: &Building<Self>) -> Self {
        Self { _source: FactSource::new(Notify::new(me, Self::on_fact)), fact: None }
    }

    fn render(&mut self, _delta_ms: u32) {
        let fact = self.fact.as_deref().unwrap_or("Loading...");
        // …
    }
}

impl FactCard {
    fn on_fact(&mut self, fact: String) {
        self.fact = Some(fact);
        request_frame();
    }
}
```

- The part does not name the widget's type, so a part in a library serves any widget.
- The part reports what changed and leaves `request_frame()` to the widget, which knows what it shows.
- A part whose data outlives one send keeps it and sends `()`: a cache the part reads again, such as decoded images, or
  state several readers share, such as a feed behind many list rows. The widget reads it when it renders.
- Name the `Notify` and the widget's method for what happened, such as `on_fact` or `on_day_change`; `on_update` is for
  a part whose state changed as a whole.
- A widget told of a reply it cannot use may call `mark_stale()` on the part's poll from the method the `Notify` runs,
  if the part exposes its `Poll`. The reply then counts as no refresh after all, but the next fetch keeps its schedule:
  only the part's reply method retries sooner, by returning `Verdict::Retry`.
- A send is delivered once the part's callback has returned, never inside it. The widget can then look into the part
  that sent; called at once, it would find it still borrowed.
- Every send is delivered, in order. One whose receiver is gone by then runs nothing.
- Like a method poll, a send needs an SDK export around it, and outside one the SDK refuses with a `BUG:` panic.

## Two Rules

**A part is not reached from its own callback.**

- While the value's method runs, the value is borrowed mutably, so the part refuses to lend it until the method returns.
- Ownership makes this hard to hit: the widget holds the part, and the part's method has no path to the widget. It takes
  a part shared outside the widget, in a `thread_local!` or an `Rc` its own value can reach.
- Reached that way, the part panics with `BUG: a Part reached from its own callback` rather than alias the value.

**The SDK never calls into the widget from inside a method.**

- A method callback runs from the top of a host-called export, never from inside an SDK call the widget made, so a
  method never has its own instance delivered to under it.
- SDK work that has to call back, such as a poll building its request after `enable_after` or `invalidate`, waits until
  the method that called it returns, then runs before the host gets control back, on that export's fuel budget. Of
  several requests a poll got by then, the last one wins, though neither `enable_after` nor `set_enabled(true)` replaces
  one on an enabled poll, and `Poll::enabled()` in the same method reflects the request, not a `build` that has not run.
- A method-callback poll therefore needs an SDK export around it: drive it from `#[widget]` hooks or SDK callbacks. From
  a hand-written export nothing would run the queued `build`, so the SDK refuses with a `BUG:` panic. A poll registered
  with the free `register_poll` builds at once, as before.

## Traps

- A method that runs out of fuel or panics traps, and the wasm stack unwinds nothing. The trap can land mid-update, so
  the instance never runs again.
- The host tears the widget down and never calls into it again; the supervisor starts a new instance after its crash
  backoff. The value the trap left borrowed is never reached again, so the SDK keeps no state of its own about traps.
- Nothing of the trapped instance carries over, and its `unload` does not run.
- A trap outside any method, in a plain-function callback or poll `build` or in SDK code between methods, is no safer:
  it may have left the widget's own state or a poll half-updated, and it ends the instance the same way.
- A trap in `init` fails the widget's instantiation.
