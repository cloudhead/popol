//!
//! Minimal non-blocking I/O library.
//!
#![deny(missing_docs)]
#![allow(clippy::new_without_default)]
#![allow(clippy::comparison_chain)]
use std::io;
use std::io::prelude::*;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time;

#[allow(non_camel_case_types)]
#[cfg(target_os = "linux")]
type nfds_t = libc::c_ulong;

#[allow(non_camel_case_types)]
#[cfg(target_os = "macos")]
type nfds_t = libc::c_uint;

pub use events::Events;

/// Source readiness events.
pub mod events {
    /// Events that can be waited for.
    pub type Events = libc::c_short;

    /// The associated file is ready to be read.
    pub const READ: Events = POLLIN | POLLPRI;
    /// The associated file is ready to be written.
    pub const WRITE: Events = POLLOUT | libc::POLLWRBAND;
    /// The associated file is ready to be read or written to.
    pub const ALL: Events = READ | WRITE;
    /// Don't wait for any events.
    pub const NONE: Events = 0x0;

    /// The associated file is available for read operations.
    pub(super) const POLLIN: Events = libc::POLLIN;
    /// There is urgent data available for read operations.
    pub(super) const POLLPRI: Events = libc::POLLPRI;
    /// The associated file is available for write operations.
    pub(super) const POLLOUT: Events = libc::POLLOUT;
    /// Error condition happened on the associated file descriptor.
    /// `poll` will always wait for this event; it is not necessary to set it.
    pub(super) const POLLERR: Events = libc::POLLERR;
    /// Hang up happened on the associated file descriptor.
    /// `poll` will always wait for this event; it is not necessary to set it.
    pub(super) const POLLHUP: Events = libc::POLLHUP;
    /// The associated file is invalid.
    /// `poll` will always wait for this event; it is not necessary to set it.
    pub(super) const POLLNVAL: Events = libc::POLLNVAL;
}

/// A source readiness event.
#[derive(Debug)]
pub struct Event<'a> {
    /// The file is writable.
    pub writable: bool,
    /// The file is readable.
    pub readable: bool,
    /// The file has be disconnected.
    pub hangup: bool,
    /// An error has occured on the file.
    pub errored: bool,
    /// The underlying source.
    pub source: &'a mut Source,
}

impl<'a> Event<'a> {
    /// Return the source from the underlying raw file descriptor.
    pub fn source<T: FromRawFd>(&self) -> T {
        unsafe { T::from_raw_fd(self.source.fd) }
    }
}

impl<'a> From<&'a mut Source> for Event<'a> {
    fn from(source: &'a mut Source) -> Self {
        let revents = source.revents;

        Self {
            readable: revents & events::READ != 0,
            writable: revents & events::WRITE != 0,
            hangup: revents & events::POLLHUP != 0,
            errored: revents & (events::POLLERR | events::POLLNVAL) != 0,
            source,
        }
    }
}

/// A source of readiness events, eg. a `net::TcpStream`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Source {
    fd: RawFd,
    events: Events,
    revents: Events,
}

impl Source {
    fn new(fd: RawFd, events: Events) -> Self {
        Self {
            fd,
            events,
            revents: 0,
        }
    }

    /// Set events to wait for on this source.
    pub fn set(&mut self, events: Events) {
        self.events |= events;
    }

    /// Unset events to wait for on this source.
    pub fn unset(&mut self, events: Events) {
        self.events &= !events;
    }
}

/// Keeps track of sources to poll.
pub struct Sources<K> {
    /// Tracks the keys assigned to each source.
    index: Vec<K>,
    /// List of sources passed to `poll`.
    list: Vec<Source>,
}

impl<K: Eq + Clone> Sources<K> {
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

    /// Register a new source, with the given key, and wait for the specified events.
    pub fn register(&mut self, key: K, fd: &impl AsRawFd, events: Events) {
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
    pub fn set(&mut self, key: &K, events: Events) -> bool {
        if let Some(ix) = self.find(key) {
            self.list[ix].set(events);
            return true;
        }
        false
    }

    /// Get a source by key.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut Source> {
        self.find(key).map(move |ix| &mut self.list[ix])
    }

    fn find(&self, key: &K) -> Option<usize> {
        self.index.iter().position(|k| k == key)
    }

    fn insert(&mut self, key: K, source: Source) {
        self.index.push(key);
        self.list.push(source);
    }
}

/// Returned by `popol::wait`.
pub enum Wait<'a, K> {
    /// Waiting for events has timed out.
    Timeout,
    /// Some sources are ready for reading or writing.
    Ready(Box<dyn Iterator<Item = (K, Event<'a>)> + 'a>),
}

impl<'a, K: 'a> Wait<'a, K> {
    /// Iterate over the readiness events.
    pub fn iter(self) -> Box<dyn Iterator<Item = (K, Event<'a>)> + 'a> {
        match self {
            Self::Ready(iter) => iter,
            Self::Timeout => Box::new(std::iter::empty()),
        }
    }

    /// Check whether there are any readiness events.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Timeout)
    }
}

/// Wakers are used to wake up `wait`.
pub struct Waker {
    reader: UnixStream,
    writer: UnixStream,
}

impl Waker {
    /// Create a new `Waker`.
    ///
    /// # Examples
    ///
    /// Wake a `wait` call from another thread.
    ///
    /// ```
    /// fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     use std::thread;
    ///     use std::time::Duration;
    ///     use std::sync::Arc;
    ///
    ///     use popol::{Sources, Waker};
    ///
    ///     const WAKER: &'static str = "waker";
    ///
    ///     let mut sources = Sources::new();
    ///
    ///     // Create a waker and keep it alive until the end of the program, so that
    ///     // the reading end doesn't get closed.
    ///     let waker = Arc::new(Waker::new(&mut sources, WAKER)?);
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
    ///     let result = popol::wait(&mut sources, Duration::from_secs(1))?;
    ///
    ///     assert!(!result.is_empty(), "There should be at least one event selected");
    ///
    ///     let mut events = result.iter();
    ///     let (key, event) = events.next().unwrap();
    ///
    ///     assert!(key == WAKER, "The event is triggered by the waker");
    ///     assert!(event.readable, "The event is readable");
    ///     assert!(events.next().is_none(), "There was only one event");
    ///
    ///     handle.join().unwrap();
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn new<K: Eq + Clone>(sources: &mut Sources<K>, key: K) -> io::Result<Waker> {
        let (writer, reader) = UnixStream::pair()?;
        let fd = reader.as_raw_fd();

        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;

        sources.insert(key, Source::new(fd, events::READ));

        Ok(Waker { reader, writer })
    }

    /// Wake up a waker. Causes `popol::wait` to return with a readiness
    /// event for this waker.
    pub fn wake(&self) -> io::Result<()> {
        use io::ErrorKind::*;

        match (&self.writer).write_all(&[0x1]) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == WouldBlock => {
                self.unblock()?;
                self.wake()
            }
            Err(e) if e.kind() == Interrupted => self.wake(),
            Err(e) => Err(e),
        }
    }

    /// Unblock the waker by draining the receive buffer.
    fn unblock(&self) -> io::Result<()> {
        let mut buf = [0; 4096];

        loop {
            match (&self.reader).read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(_) => continue,

                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }
}

/// Wait for readiness events on the given list of sources. If no event
/// is returned within the given timeout, returns `Wait::Timeout`.
pub fn wait<'a, K: Eq + Clone>(
    sources: &'a mut Sources<K>,
    timeout: time::Duration,
) -> Result<Wait<'a, K>, io::Error> {
    let timeout = timeout.as_millis() as libc::c_int;

    let result = unsafe {
        libc::poll(
            sources.list.as_mut_ptr() as *mut libc::pollfd,
            sources.list.len() as nfds_t,
            timeout,
        )
    };

    if result == 0 {
        Ok(Wait::Timeout)
    } else if result > 0 {
        Ok(Wait::Ready(Box::new(
            sources
                .index
                .iter()
                .zip(sources.list.iter_mut())
                .filter(|(_, d)| d.revents != 0)
                .map(|(key, source)| (key.clone(), Event::from(source))),
        )))
    } else {
        Err(io::Error::last_os_error())
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

        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1, &reader2] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, events::READ);
        sources.register("reader1", &reader1, events::READ);
        sources.register("reader2", &reader2, events::READ);

        {
            let result = wait(&mut sources, Duration::from_millis(1))?;
            assert!(result.is_empty());
        }

        let tests = &mut [
            (&writer0, &reader0, "reader0", 0x1 as u8),
            (&writer1, &reader1, "reader1", 0x2 as u8),
            (&writer2, &reader2, "reader2", 0x3 as u8),
        ];

        for (mut writer, mut reader, key, byte) in tests.iter_mut() {
            let mut buf = [0u8; 1];

            assert!(matches!(
                reader.read(&mut buf[..]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock
            ));

            writer.write(&[*byte])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            assert!(!result.is_empty());

            let mut events = result.iter();
            let (k, event) = events.next().unwrap();

            assert_eq!(&k, key);
            assert!(event.readable && !event.writable && !event.errored && !event.hangup);
            assert!(events.next().is_none());

            assert_eq!(reader.read(&mut buf[..])?, 1);
            assert_eq!(&buf[..], &[*byte]);
        }
        Ok(())
    }

    #[test]
    fn test_threaded() -> io::Result<()> {
        let (writer0, reader0) = UnixStream::pair()?;
        let (writer1, reader1) = UnixStream::pair()?;
        let (writer2, reader2) = UnixStream::pair()?;

        let mut sources = Sources::new();
        let readers = &[&reader0, &reader1, &reader2];

        for reader in readers {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, events::READ);
        sources.register("reader1", &reader1, events::READ);
        sources.register("reader2", &reader2, events::READ);

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(8));

            for writer in &mut [&writer1, &writer2, &writer0] {
                writer.write(&[1]).unwrap();
                writer.write(&[2]).unwrap();
            }
        });

        let mut closed = vec![];
        while closed.len() < readers.len() {
            let result = wait(&mut sources, Duration::from_millis(64))?;

            assert!(!result.is_empty());

            for (key, event) in result.iter() {
                assert!(event.readable);
                assert!(!event.writable);
                assert!(!event.errored);

                if event.hangup {
                    closed.push(key);
                    continue;
                }

                let mut buf = [0u8; 2];
                let mut reader = match key {
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

        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1, &reader2] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, events::READ);
        sources.register("reader1", &reader1, events::READ);
        sources.register("reader2", &reader2, events::READ);

        {
            let result = wait(&mut sources, Duration::from_millis(1))?;
            assert!(result.is_empty());
        }

        {
            writer1.write(&[0x0])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            let (key, _) = result.iter().next().unwrap();

            assert_eq!(key, "reader1");
        }

        // Unregister.
        {
            sources.unregister(&"reader1");
            writer1.write(&[0x0])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            assert!(result.iter().next().is_none());

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write(&[0])?;
            }

            let result = wait(&mut sources, Duration::from_millis(1))?;
            let keys = result.iter().map(|(k, _)| k).collect::<HashSet<_>>();

            assert!(keys.contains("reader0"));
            assert!(!keys.contains("reader1"));
            assert!(keys.contains("reader2"));

            sources.unregister(&"reader0");

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write(&[0])?;
            }

            let result = wait(&mut sources, Duration::from_millis(1))?;
            let keys = result.iter().map(|(k, _)| k).collect::<HashSet<_>>();

            assert!(!keys.contains("reader0"));
            assert!(!keys.contains("reader1"));
            assert!(keys.contains("reader2"));

            sources.unregister(&"reader2");

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write(&[0])?;
            }

            let result = wait(&mut sources, Duration::from_millis(1))?;

            assert!(result.is_empty());
        }

        // Re-register.
        {
            sources.register("reader0", &reader0, events::READ);
            writer0.write(&[0])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            let (key, _) = result.iter().next().unwrap();

            assert_eq!(key, "reader0");
        }

        Ok(())
    }

    #[test]
    fn test_set() -> io::Result<()> {
        let (mut writer0, reader0) = UnixStream::pair()?;
        let (mut writer1, reader1) = UnixStream::pair()?;

        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, events::READ);
        sources.register("reader1", &reader1, events::NONE);

        {
            writer0.write(&[0])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            let (key, event) = result.iter().next().unwrap();
            assert_eq!(key, "reader0");

            event.source.unset(events::READ);
            writer0.write(&[0])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            assert!(result.iter().next().is_none());
        }

        {
            writer1.write(&[0])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            assert!(result.iter().next().is_none());

            sources.set(&"reader1", events::READ);
            writer1.write(&[0])?;

            let result = wait(&mut sources, Duration::from_millis(1))?;
            let (key, _) = result.iter().next().unwrap();
            assert_eq!(key, "reader1");
        }

        Ok(())
    }

    #[test]
    fn test_waker() -> io::Result<()> {
        let mut sources = Sources::new();
        let mut waker = Waker::new(&mut sources, "waker")?;
        let buf = [0; 4096];

        let result = wait(&mut sources, Duration::from_millis(1))?;
        assert!(result.iter().next().is_none());

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

        let result = wait(&mut sources, Duration::from_millis(1))?;
        let (key, event) = result.iter().next().unwrap();

        assert!(event.readable);
        assert!(!event.writable && !event.hangup && !event.errored);
        assert_eq!(key, "waker");

        waker.wake()?;

        let result = wait(&mut sources, Duration::from_millis(1))?;
        let (key, event) = result.iter().next().unwrap();

        assert!(event.readable);
        assert_eq!(key, "waker");

        // Try to wake multiple times.
        waker.wake()?;
        waker.wake()?;
        waker.wake()?;

        let result = wait(&mut sources, Duration::from_millis(1))?;
        assert_eq!(result.iter().count(), 1, "multiple wakes count as one");

        Ok(())
    }
}
