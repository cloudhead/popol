# popol

*An minimal  ergonomic wrapper around `poll()`, built for peer-to-peer
networking.*

Async I/O in Rust is still an unsolved problem. With the stabilization of
*async/await*, we are seeing a number of libraries and runtimes crop up, such
as *async-std* and *smol*, while others such as *tokio* and *mio* are maturing.
The problem with *async/await* is that you can't use any of the standard
library traits, such as `Read` and `Write`.  The async ecosystem comes with an
entirely separate suite of traits (eg. `AsyncRead` and `AsyncWrite`) and I/O
libraries. Furthermore, most of these runtimes have a large dependency
footprint, partly from having to provide async alternatives to the standard
library functions, and partly due to the complexity of these runtimes.
Finally, *async/await* has a very steep learning curve and has a lot of
rough edges.

What do we need? For most use-cases, the ability to handle between a dozen
and up to a few hundred open connections without blocking, is all we need.
This places us squarely within the territory of the venerable `poll()` function,
which is available on almost all platforms.

By building on `poll`, we have the following advantages:

* Much smaller implementation than even the smallest *async/await* runtimes
* All of the Rust standard library just works
* Close to zero-dependency (*popol* depends solely on *libc* and *bitflags*)
* No "runtime" keeps the code much easier to reason about

Why not use `epoll`? A couple of reasons:

1. It is more complex than `poll` and requires us to write more code
2. It isn't portable (only works on Linux)
3. `poll` is sufficient for handling most workloads

Why not *mio*? It is quite a bit more complex, and comes with its own
socket library which wraps all of the standard library types.
If that doesn't bother you, it's a good choice!
