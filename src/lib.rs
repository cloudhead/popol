//!
//! Minimal non-blocking I/O library.
//!
//! ## Example: reading from *stdin*
//!
//! ```
//! use std::{io, io::prelude::*, process, time};
//!
//! fn main() -> io::Result<()> {
//!     // Create a registry to hold I/O sources.
//!     let mut sources = popol::Sources::with_capacity(1);
//!     // Create an events buffer to hold readiness events.
//!     let mut events = Vec::with_capacity(1);
//!
//!     // Register the program's standard input as a source of "read" readiness events.
//!     sources.register((), &io::stdin(), popol::interest::READ);
//!
//!     // Wait on our event sources for at most 6 seconds. If an event source is
//!     // ready before then, process its events. Otherwise, timeout.
//!     match sources.poll(&mut events, popol::Timeout::from_secs(6)) {
//!         Ok(_) => {}
//!         Err(err) if err.kind() == io::ErrorKind::TimedOut => process::exit(1),
//!         Err(err) => return Err(err),
//!     }
//!
//!     // Iterate over source events. Since we only have one source
//!     // registered, this will only iterate once.
//!     for event in events.drain(..) {
//!         // The standard input has data ready to be read.
//!         if event.is_readable() || event.is_hangup() {
//!             let mut buf = [0; 1024];
//!
//!             // Read what we can from standard input and echo it.
//!             match io::stdin().read(&mut buf[..]) {
//!                 Ok(n) => io::stdout().write_all(&buf[..n])?,
//!                 Err(err) => panic!("{}", err),
//!             }
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
#![deny(missing_docs)]
#![allow(clippy::new_without_default)]
#![allow(clippy::comparison_chain)]
use std::io;
use std::io::prelude::*;
use std::ops::Deref;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub use interest::Interest;

/// Raw input or output events.
pub type Events = libc::c_short;

/// Source readiness interest.
pub mod interest {
    /// Events that can be waited for.
    pub type Interest = super::Events;

    /// The associated file is ready to be read.
    pub const READ: Interest = POLLIN | POLLPRI;
    /// The associated file is ready to be written.
    pub const WRITE: Interest = POLLOUT | libc::POLLWRBAND;
    /// The associated file is ready.
    pub const ALL: Interest = READ | WRITE;
    /// Don't wait for any events.
    pub const NONE: Interest = 0x0;

    // NOTE: POLLERR, POLLNVAL and POLLHUP are ignored as *interests*, and will
    // always be set automatically in the output events.

    /// The associated file is available for read operations.
    const POLLIN: Interest = libc::POLLIN;
    /// There is urgent data available for read operations.
    const POLLPRI: Interest = libc::POLLPRI;
    /// The associated file is available for write operations.
    const POLLOUT: Interest = libc::POLLOUT;
}

/// An I/O ready event.
#[derive(Debug)]
pub struct Event<K> {
    /// The event key.
    pub key: K,
    /// The source of the event.
    pub source: Source,
}

impl<K> Deref for Event<K> {
    type Target = Source;

    fn deref(&self) -> &Self::Target {
        &self.source
    }
}

/// Optional timeout.
///
/// Note that the maximum timeout is `i32::MAX` milliseconds (about 25 days). Longer
/// timeouts will be silently clipped to `i32::MAX` milliseconds.
#[derive(Debug, Clone)]
pub enum Timeout {
    /// Timeout after a specific duration.
    After(Duration),
    /// Never timeout.
    Never,
}

impl Timeout {
    /// Create a timeout with the specified number of seconds.
    ///
    /// See [`Timeout`] for an important note about the maximum timeout.
    pub fn from_secs(seconds: u32) -> Self {
        Self::After(Duration::from_secs(seconds as u64))
    }

    /// Create a timeout with the specified number of milliseconds.
    ///
    /// See [`Timeout`] for an important note about the maximum timeout.
    pub fn from_millis(milliseconds: u32) -> Self {
        Self::After(Duration::from_millis(milliseconds as u64))
    }
}

impl From<Duration> for Timeout {
    /// Create a timeout from a duration.
    ///
    /// See [`Timeout`] for an important note about the maximum timeout.
    fn from(duration: Duration) -> Self {
        Self::After(duration)
    }
}

impl From<Option<Duration>> for Timeout {
    /// Create a timeout from an optional duration.
    ///
    /// See [`Timeout`] for an important note about the maximum timeout.
    fn from(duration: Option<Duration>) -> Self {
        match duration {
            Some(duration) => Self::from(duration),
            None => Self::Never,
        }
    }
}

/// A source of readiness events, eg. a `net::TcpStream`.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct Source {
    fd: RawFd,
    events: Interest,
    revents: Interest,
}

impl Source {
    fn new(fd: impl AsRawFd, events: Interest) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            events,
            revents: 0,
        }
    }

    /// Return the source from the underlying raw file descriptor.
    ///
    /// # Safety
    ///
    /// Calls [`FromRawFd::from_raw_fd`]. The returned object will cause
    /// the file to close when dropped.
    pub unsafe fn raw<T: FromRawFd>(&self) -> T {
        T::from_raw_fd(self.fd)
    }

    /// Set events to wait for on this source.
    pub fn set(&mut self, events: Interest) {
        self.events |= events;
    }

    /// Unset events to wait for on this source.
    pub fn unset(&mut self, events: Interest) {
        self.events &= !events;
    }

    /// Returns raw representation of events which fired during poll.
    pub fn raw_events(&self) -> Events {
        self.revents
    }

    /// The source is writable.
    pub fn is_writable(self) -> bool {
        self.revents & interest::WRITE != 0
    }

    /// The source is readable.
    pub fn is_readable(self) -> bool {
        self.revents & interest::READ != 0
    }

    /// The source has been disconnected.
    pub fn is_hangup(self) -> bool {
        self.revents & libc::POLLHUP != 0
    }

    /// An error has occurred on the source.
    ///
    /// Note that this function is best used in combination with
    /// [`Self::is_invalid`], to detect all error cases.
    pub fn is_error(self) -> bool {
        self.revents & libc::POLLERR != 0
    }

    /// The source is not valid.
    pub fn is_invalid(self) -> bool {
        self.revents & libc::POLLNVAL != 0
    }
}

impl AsRawFd for &Source {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl AsRawFd for Source {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

/// Keeps track of sources to poll.
#[derive(Debug, Clone)]
pub struct Sources<K> {
    /// Tracks the keys assigned to each source.
    index: Vec<K>,
    /// List of sources passed to `poll`.
    list: Vec<Source>,
}

impl<K> Sources<K> {
    /// Creates a new set of sources to poll.
    pub fn new() -> Self {
        Self {
            index: vec![],
            list: vec![],
        }
    }

    /// Creates a new set of sources to poll, with the given capacity.
    /// Use this if you have a lot of sources to poll.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            index: Vec::with_capacity(cap),
            list: Vec::with_capacity(cap),
        }
    }

    /// Return the number of registered sources.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Return whether the source registry is empty.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

impl<S: AsRawFd, K: PartialEq + Eq + Clone> FromIterator<(K, S, Interest)> for Sources<K> {
    fn from_iter<T: IntoIterator<Item = (K, S, Interest)>>(iter: T) -> Self {
        let mut sources = Sources::new();
        for (key, source, interest) in iter {
            sources.register(key, &source, interest);
        }
        sources
    }
}

impl<K: Clone + PartialEq> Sources<K> {
    /// Register a new source, with the given key, and wait for the specified events.
    ///
    /// Care must be taken not to register the same source twice, or use the same key
    /// for two different sources.
    pub fn register(&mut self, key: K, fd: &impl AsRawFd, events: Interest) {
        self.insert(key, Source::new(fd.as_raw_fd(), events));
    }

    /// Unregister a  source, given its key.
    pub fn unregister(&mut self, key: &K) {
        if let Some(ix) = self.find(key) {
            self.index.swap_remove(ix);
            self.list.swap_remove(ix);
        }
    }

    /// Set the events to poll for on a source identified by its key.
    pub fn set(&mut self, key: &K, events: Interest) -> bool {
        if let Some(ix) = self.find(key) {
            self.list[ix].set(events);
            return true;
        }
        false
    }

    /// Unset event interests on a source.
    pub fn unset(&mut self, key: &K, events: Interest) -> bool {
        if let Some(ix) = self.find(key) {
            self.list[ix].unset(events);
            return true;
        }
        false
    }

    /// Get a source by key.
    pub fn get(&mut self, key: &K) -> Option<&Source> {
        self.find(key).map(move |ix| &self.list[ix])
    }

    /// Get a source by key, mutably.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut Source> {
        self.find(key).map(move |ix| &mut self.list[ix])
    }

    /// Wait for readiness events on the given list of sources. If no event
    /// is returned within the given timeout, returns an error of kind [`io::ErrorKind::TimedOut`].
    ///
    /// This is identical to [`Self::wait()`] and [`Self::wait_timeout()`] except that the timeout
    /// is optional.
    ///
    /// New events will be appended to the events buffer. Make sure to clear the buffer before
    /// calling this function, if necessary.
    pub fn poll(
        &mut self,
        events: &mut impl Extend<Event<K>>,
        timeout: impl Into<Timeout>,
    ) -> Result<usize, io::Error> {
        let timeout = match timeout.into() {
            Timeout::After(duration) => duration.as_millis() as libc::c_int,
            Timeout::Never => -1,
        };
        // Exit if there's nothing to poll.
        if self.list.is_empty() {
            return Ok(0);
        }

        loop {
            // SAFETY: required for FFI; shouldn't break rust guarantees.
            let result = unsafe {
                libc::poll(
                    self.list.as_mut_ptr() as *mut libc::pollfd,
                    self.list.len() as libc::nfds_t,
                    timeout,
                )
            };

            events.extend(
                self.index
                    .iter()
                    .zip(self.list.iter())
                    .filter(|(_, s)| s.revents != 0)
                    .map(|(key, source)| Event {
                        key: key.clone(),
                        source: *source,
                    }),
            );

            if result == 0 {
                if self.is_empty() {
                    return Ok(0);
                } else {
                    return Err(io::ErrorKind::TimedOut.into());
                }
            } else if result > 0 {
                return Ok(result as usize);
            } else {
                let err = io::Error::last_os_error();
                match err.raw_os_error() {
                    // Poll can fail if "The allocation of internal data structures failed". But
                    // a subsequent request may succeed.
                    Some(libc::EAGAIN) => continue,
                    // Poll can also fail if it received an interrupt. It's a good idea to retry
                    // in that case.
                    Some(libc::EINTR) => continue,
                    _ => {
                        return Err(err);
                    }
                }
            }
        }
    }

    /// Wait for readiness events on the given list of sources. If no event
    /// is returned within the given timeout, returns an error of kind [`io::ErrorKind::TimedOut`].
    ///
    /// This is identical to [`Self::poll()`] and [`Self::wait()`], except that you must specify a
    /// timeout with this.
    pub fn wait_timeout(
        &mut self,
        events: &mut impl Extend<Event<K>>,
        timeout: Duration,
    ) -> Result<usize, io::Error> {
        self.poll(events, timeout)
    }

    /// Wait for readiness events on the given list of sources, or until the call
    /// is interrupted.
    ///
    /// This is identical to [`Self::poll()`] and [`Self::wait_timeout()`] except that you cannot
    /// specify a timeout with this.
    pub fn wait(&mut self, events: &mut impl Extend<Event<K>>) -> Result<usize, io::Error> {
        self.poll(events, Timeout::Never)
    }

    fn find(&self, key: &K) -> Option<usize> {
        self.index.iter().position(|k| k == key)
    }

    fn insert(&mut self, key: K, source: Source) {
        self.index.push(key);
        self.list.push(source);
    }
}

/// Wakers are used to wake up `wait`.
pub struct Waker {
    reader: UnixStream,
    writer: UnixStream,
}

impl AsRawFd for &Waker {
    fn as_raw_fd(&self) -> RawFd {
        self.reader.as_raw_fd()
    }
}

impl Waker {
    /// Create a new `Waker` and register it.
    ///
    /// # Examples
    ///
    /// Wake a `poll` call from another thread.
    ///
    /// ```
    /// fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     use std::thread;
    ///     use std::time::Duration;
    ///     use std::sync::Arc;
    ///
    ///     use popol::{Event, Sources, Waker, Timeout};
    ///
    ///     const WAKER: &'static str = "waker";
    ///
    ///     let mut events = Vec::new();
    ///     let mut sources = Sources::new();
    ///
    ///     // Create a waker and keep it alive until the end of the program, so that
    ///     // the reading end doesn't get closed.
    ///     let waker = Arc::new(Waker::register(&mut sources, WAKER)?);
    ///     let _waker = waker.clone();
    ///
    ///     let handle = thread::spawn(move || {
    ///         thread::sleep(Duration::from_millis(160));
    ///
    ///         // Wake up popol on the main thread.
    ///         _waker.wake().expect("waking shouldn't fail");
    ///     });
    ///
    ///     // Wait to be woken up by the other thread. Otherwise, time out.
    ///     sources.poll(&mut events, Timeout::from_secs(1))?;
    ///
    ///     assert!(!events.is_empty(), "There should be at least one event selected");
    ///
    ///     let mut events = events.iter();
    ///     let Event { key, source } = events.next().unwrap();
    ///
    ///     assert!(key == &WAKER, "The event is triggered by the waker");
    ///     assert!(source.is_readable(), "The event is readable");
    ///     assert!(events.next().is_none(), "There was only one event");
    ///
    ///     handle.join().unwrap();
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn register<K: Eq + Clone>(sources: &mut Sources<K>, key: K) -> io::Result<Waker> {
        let waker = Waker::new()?;
        sources.insert(key, Source::new(&waker, interest::READ));

        Ok(waker)
    }

    /// Create a new waker.
    pub fn new() -> io::Result<Waker> {
        let (writer, reader) = UnixStream::pair()?;

        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;

        Ok(Waker { reader, writer })
    }

    /// Wake up a waker. Causes `popol::wait` to return with a readiness
    /// event for this waker.
    pub fn wake(&self) -> io::Result<()> {
        use io::ErrorKind::*;

        match (&self.writer).write_all(&[0x1]) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == WouldBlock => {
                Waker::reset(self.reader.as_raw_fd())?;
                self.wake()
            }
            Err(e) if e.kind() == Interrupted => self.wake(),
            Err(e) => Err(e),
        }
    }

    /// Reset the waker by draining the receive buffer.
    pub fn reset(fd: impl AsRawFd) -> io::Result<()> {
        let mut buf = [0u8; 4096];

        loop {
            // We use a low-level "read" here because the alternative is to create a `UnixStream`
            // from the `RawFd`, which has "drop" semantics which we want to avoid.
            match unsafe {
                libc::read(
                    fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            } {
                -1 => match io::Error::last_os_error() {
                    e if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                    e => return Err(e),
                },
                0 => return Ok(()),
                _ => continue,
            }
        }
    }
}

/// Set non-blocking mode on a stream.
///
/// This is a convenience function if the source of your stream doesn't provide an
/// easy way to set it into non-blocking mode.
///
/// ## Example
///
/// ```
/// use std::process;
/// use popol::set_nonblocking;
///
/// let child = process::Command::new("ls")
///     .stdout(process::Stdio::piped())
///     .spawn()
///     .unwrap();
/// let out = child.stdout.unwrap();
///
/// set_nonblocking(&out, true).unwrap();
/// ```
///
/// ## Return
///
/// On Linux, this should always return `Ok(0)` or `Err(_)`. On other operating systems,
/// consult the `fcntl(2)` man page.
pub fn set_nonblocking(fd: &dyn AsRawFd, nonblocking: bool) -> io::Result<i32> {
    let fd = fd.as_raw_fd();

    // SAFETY: required for FFI; shouldn't break rust guarantees.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }

    let flags = if nonblocking {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };

    // SAFETY: required for FFI; shouldn't break rust guarantees.
    match unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } {
        -1 => Err(io::Error::last_os_error()),
        result => Ok(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_readable() -> io::Result<()> {
        let (writer0, reader0) = UnixStream::pair()?;
        let (writer1, reader1) = UnixStream::pair()?;
        let (writer2, reader2) = UnixStream::pair()?;

        let mut events = Vec::new();
        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1, &reader2] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, interest::READ);
        sources.register("reader1", &reader1, interest::READ);
        sources.register("reader2", &reader2, interest::READ);

        {
            let err = sources
                .poll(&mut events, Timeout::from_millis(1))
                .unwrap_err();

            assert_eq!(err.kind(), io::ErrorKind::TimedOut);
            assert!(events.is_empty());
        }

        let tests = &mut [
            (&writer0, &reader0, "reader0", 0x1u8),
            (&writer1, &reader1, "reader1", 0x2u8),
            (&writer2, &reader2, "reader2", 0x3u8),
        ];

        for (mut writer, mut reader, key, byte) in tests.iter_mut() {
            let mut buf = [0u8; 1];

            assert!(matches!(
                reader.read(&mut buf[..]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock
            ));

            writer.write_all(&[*byte])?;

            events.clear();
            sources.poll(&mut events, Timeout::from_millis(1))?;
            assert!(!events.is_empty());

            let mut events = events.iter();
            let event = events.next().unwrap();

            assert_eq!(&event.key, key);
            assert!(
                event.is_readable()
                    && !event.is_writable()
                    && !event.is_error()
                    && !event.is_hangup()
            );
            assert!(events.next().is_none());

            assert_eq!(reader.read(&mut buf[..])?, 1);
            assert_eq!(&buf[..], &[*byte]);
        }
        Ok(())
    }

    #[test]
    fn test_empty() -> io::Result<()> {
        let mut events: Vec<Event<()>> = Vec::new();
        let mut sources = Sources::new();

        sources
            .poll(&mut events, Timeout::from_millis(1))
            .expect("no error if nothing registered");

        assert!(events.is_empty());

        Ok(())
    }

    #[test]
    fn test_timeout() -> io::Result<()> {
        let mut events = Vec::new();
        let mut sources = Sources::new();

        sources.register((), &io::stdout(), interest::READ);

        let err = sources
            .poll(&mut events, Timeout::from_millis(1))
            .unwrap_err();

        assert_eq!(sources.len(), 1);
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(events.is_empty());

        Ok(())
    }

    #[test]
    fn test_threaded() -> io::Result<()> {
        let (writer0, reader0) = UnixStream::pair()?;
        let (writer1, reader1) = UnixStream::pair()?;
        let (writer2, reader2) = UnixStream::pair()?;

        let mut events = Vec::new();
        let mut sources = Sources::new();
        let readers = &[&reader0, &reader1, &reader2];

        for reader in readers {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, interest::READ);
        sources.register("reader1", &reader1, interest::READ);
        sources.register("reader2", &reader2, interest::READ);

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(8));

            for writer in &mut [&writer1, &writer2, &writer0] {
                writer.write_all(&[1]).unwrap();
                writer.write_all(&[2]).unwrap();
            }
        });

        let mut closed = vec![];
        while closed.len() < readers.len() {
            sources.poll(&mut events, Timeout::from_millis(64))?;

            for event in events.drain(..) {
                assert!(event.is_readable());
                assert!(!event.is_writable());
                assert!(!event.is_error());

                if event.is_hangup() {
                    closed.push(event.key.to_owned());
                    continue;
                }

                let mut buf = [0u8; 2];
                let mut reader = match event.key {
                    "reader0" => &reader0,
                    "reader1" => &reader1,
                    "reader2" => &reader2,
                    _ => unreachable!(),
                };
                let n = reader.read(&mut buf[..])?;

                assert_eq!(n, 2);
                assert_eq!(&buf[..], &[1, 2]);
            }
        }
        handle.join().unwrap();

        Ok(())
    }

    #[test]
    fn test_unregister() -> io::Result<()> {
        use std::collections::HashSet;

        let (mut writer0, reader0) = UnixStream::pair()?;
        let (mut writer1, reader1) = UnixStream::pair()?;
        let (writer2, reader2) = UnixStream::pair()?;

        let mut events = Vec::new();
        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1, &reader2] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, interest::READ);
        sources.register("reader1", &reader1, interest::READ);
        sources.register("reader2", &reader2, interest::READ);

        {
            let err = sources
                .poll(&mut events, Timeout::from_millis(1))
                .unwrap_err();

            assert_eq!(err.kind(), io::ErrorKind::TimedOut);
            assert!(events.is_empty());
        }

        {
            writer1.write_all(&[0x0])?;

            events.clear();
            sources.poll(&mut events, Timeout::from_millis(1))?;
            let event = events.first().unwrap();

            assert_eq!(event.key, "reader1");
        }

        // Unregister.
        {
            sources.unregister(&"reader1");
            writer1.write_all(&[0x0])?;

            events.clear();
            sources.poll(&mut events, Timeout::from_millis(1)).ok();
            assert!(events.first().is_none());

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write_all(&[0])?;
            }

            sources.poll(&mut events, Timeout::from_millis(1))?;
            let keys = events.iter().map(|e| e.key).collect::<HashSet<_>>();

            assert!(keys.contains(&"reader0"));
            assert!(!keys.contains(&"reader1"));
            assert!(keys.contains(&"reader2"));

            sources.unregister(&"reader0");

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write_all(&[0])?;
            }

            events.clear();
            sources.poll(&mut events, Timeout::from_millis(1))?;
            let keys = events.iter().map(|e| e.key).collect::<HashSet<_>>();

            assert!(!keys.contains(&"reader0"));
            assert!(!keys.contains(&"reader1"));
            assert!(keys.contains(&"reader2"));

            sources.unregister(&"reader2");

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write_all(&[0])?;
            }

            events.clear();
            sources.poll(&mut events, Timeout::from_millis(1)).ok();

            assert!(events.is_empty());
        }

        // Re-register.
        {
            sources.register("reader0", &reader0, interest::READ);
            writer0.write_all(&[0])?;

            sources.poll(&mut events, Timeout::from_millis(1))?;
            let event = events.first().unwrap();

            assert_eq!(event.key, "reader0");
        }

        Ok(())
    }

    #[test]
    fn test_set() -> io::Result<()> {
        let (mut writer0, reader0) = UnixStream::pair()?;
        let (mut writer1, reader1) = UnixStream::pair()?;

        let mut events = Vec::new();
        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, interest::READ);
        sources.register("reader1", &reader1, interest::NONE);

        {
            writer0.write_all(&[0])?;

            sources.poll(&mut events, Timeout::from_millis(1))?;
            let event = events.first().unwrap();
            assert_eq!(event.key, "reader0");

            sources.unset(&event.key, interest::READ);
            writer0.write_all(&[0])?;
            events.clear();

            sources.poll(&mut events, Timeout::from_millis(1)).ok();
            assert!(events.first().is_none());
        }

        {
            writer1.write_all(&[0])?;

            sources.poll(&mut events, Timeout::from_millis(1)).ok();
            assert!(events.first().is_none());

            sources.set(&"reader1", interest::READ);
            writer1.write_all(&[0])?;

            sources.poll(&mut events, Timeout::from_millis(1))?;
            let event = events.first().unwrap();
            assert_eq!(event.key, "reader1");
        }

        Ok(())
    }

    #[test]
    fn test_waker() -> io::Result<()> {
        let mut events = Vec::new();
        let mut sources = Sources::new();
        let mut waker = Waker::register(&mut sources, "waker")?;
        let buf = [0; 4096];

        sources.poll(&mut events, Timeout::from_millis(1)).ok();
        assert!(events.first().is_none());

        // Fill the waker stream until it would block..
        loop {
            match waker.writer.write(&buf) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => return Err(e),
                _ => continue,
            }
        }

        sources.poll(&mut events, Timeout::from_millis(1))?;
        let event @ Event { key, .. } = events.first().unwrap();

        assert!(event.is_readable());
        assert!(!event.is_writable() && !event.is_hangup() && !event.is_error());
        assert_eq!(key, &"waker");

        waker.wake()?;

        events.clear();
        sources.poll(&mut events, Timeout::from_millis(1))?;
        let event @ Event { key, .. } = events.first().unwrap();

        assert!(event.is_readable());
        assert_eq!(key, &"waker");

        // Try to wake multiple times.
        waker.wake()?;
        waker.wake()?;
        waker.wake()?;

        events.clear();
        sources.poll(&mut events, Timeout::from_millis(1))?;
        assert_eq!(events.len(), 1, "multiple wakes count as one");

        let event @ Event { key, .. } = events.first().unwrap();
        assert_eq!(key, &"waker");

        Waker::reset(&event.source).unwrap();

        // Try waiting multiple times.
        let result = sources.poll(&mut events, Timeout::from_millis(1));
        assert!(
            matches!(
                result.err().map(|e| e.kind()),
                Some(io::ErrorKind::TimedOut)
            ),
            "the waker should only wake once"
        );

        Ok(())
    }

    #[test]
    fn test_waker_threaded() {
        let mut events = Vec::new();
        let mut sources = Sources::new();
        let waker = Waker::register(&mut sources, "waker").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let iterations = 100_000;
        let handle = std::thread::spawn(move || {
            for _ in 0..iterations {
                tx.send(()).unwrap();
                waker.wake().unwrap();
            }
        });

        let mut wakes = 0;
        let mut received = 0;

        while !handle.is_finished() {
            events.clear();

            let count = sources.poll(&mut events, Timeout::Never).unwrap();
            if count > 0 {
                let event = events.pop().unwrap();
                assert_eq!(event.key, "waker");
                assert!(events.is_empty());

                // There's always a message on the channel if we got woken up.
                rx.recv().unwrap();
                received += 1;

                // We may get additional messages on the channel, if the sending is
                // faster than the waking.
                while rx.try_recv().is_ok() {
                    received += 1;
                }

                if received == iterations {
                    // Error: "bad file descriptor", as the waker handle gets
                    // dropped by the other thread.
                    Waker::reset(event.source).unwrap_err();
                    break;
                }

                Waker::reset(event.source).ok(); // We might get the "bad file descriptor" error here.
                wakes += 1;
            }
        }
        handle.join().unwrap();

        assert_eq!(received, iterations);
        assert!(wakes <= received);
    }
}
