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

pub use interest::Interest;

/// Source readiness interest.
pub mod interest {
    /// Events that can be waited for.
    pub type Interest = libc::c_short;

    /// The associated file is ready to be read.
    pub const READ: Interest = POLLIN | POLLPRI;
    /// The associated file is ready to be written.
    pub const WRITE: Interest = POLLOUT | libc::POLLWRBAND;
    /// The associated file is invalid or has had an error.
    pub const ERR: Interest = POLLERR | POLLNVAL;
    /// The associated file has hung up.
    pub const HANGUP: Interest = POLLHUP;
    /// The associated file is ready.
    pub const ALL: Interest = READ | WRITE | ERR | HANGUP;
    /// Don't wait for any events.
    pub const NONE: Interest = 0x0;

    /// The associated file is available for read operations.
    pub(super) const POLLIN: Interest = libc::POLLIN;
    /// There is urgent data available for read operations.
    pub(super) const POLLPRI: Interest = libc::POLLPRI;
    /// The associated file is available for write operations.
    pub(super) const POLLOUT: Interest = libc::POLLOUT;
    /// Error condition happened on the associated file descriptor.
    /// `poll` will always wait for this event; it is not necessary to set it.
    pub(super) const POLLERR: Interest = libc::POLLERR;
    /// Hang up happened on the associated file descriptor.
    /// `poll` will always wait for this event; it is not necessary to set it.
    pub(super) const POLLHUP: Interest = libc::POLLHUP;
    /// The associated file is invalid.
    /// `poll` will always wait for this event; it is not necessary to set it.
    pub(super) const POLLNVAL: Interest = libc::POLLNVAL;
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
    pub source: &'a Source,
}

impl<'a> Event<'a> {
    /// Return the source from the underlying raw file descriptor.
    pub fn source<T: FromRawFd>(&self) -> T {
        unsafe { T::from_raw_fd(self.source.fd) }
    }
}

impl<'a> From<&'a Source> for Event<'a> {
    fn from(source: &'a Source) -> Self {
        let revents = source.revents;

        Self {
            readable: revents & interest::READ != 0,
            writable: revents & interest::WRITE != 0,
            hangup: revents & interest::HANGUP != 0,
            errored: revents & interest::ERR != 0,
            source,
        }
    }
}

/// Populated by `wait` with source readiness events.
#[derive(Debug)]
pub struct Events<K> {
    /// Number of events.
    count: usize,
    /// Sources polled.
    sources: Sources<K>,
}

impl<K: Eq + Clone> Events<K> {
    /// Create a new empty event tracker.
    pub fn new() -> Self {
        Self {
            count: 0,
            sources: Sources::new(),
        }
    }

    /// Create an empty event tracker of a certain capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            count: 0,
            sources: Sources::with_capacity(cap),
        }
    }

    /// Iterate over ready sources and their keys.
    pub fn iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, Event<'a>)> + 'a {
        self.sources
            .index
            .iter()
            .zip(self.sources.list.iter())
            .filter(|(_, d)| d.revents != 0)
            .map(|(key, source)| (key, Event::from(source)))
    }

    /// Check whether the event list is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return the number of readiness events.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Initialize the events list with sources.
    fn initialize(&mut self, sources: Sources<K>) {
        self.sources = sources;
    }
}

/// A source of readiness events, eg. a `net::TcpStream`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Source {
    fd: RawFd,
    events: Interest,
    revents: Interest,
}

impl Source {
    fn new(fd: RawFd, events: Interest) -> Self {
        Self {
            fd,
            events,
            revents: 0,
        }
    }

    /// Set events to wait for on this source.
    pub fn set(&mut self, events: Interest) {
        self.events |= events;
    }

    /// Unset events to wait for on this source.
    pub fn unset(&mut self, events: Interest) {
        self.events &= !events;
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

    /// Return the number of registered sources.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Return whether the source registry is empty.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Register a new source, with the given key, and wait for the specified events.
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
    pub fn get_mut(&mut self, key: &K) -> Option<&mut Source> {
        self.find(key).map(move |ix| &mut self.list[ix])
    }

    /// Wait for readiness events on the given list of sources. If no event
    /// is returned within the given timeout, returns `Wait::Timeout`.
    pub fn wait<'a>(
        &mut self,
        events: &mut Events<K>,
        timeout: time::Duration,
    ) -> Result<(), io::Error> {
        let timeout = timeout.as_millis() as libc::c_int;

        events.initialize(self.clone());

        let result = unsafe {
            libc::poll(
                events.sources.list.as_mut_ptr() as *mut libc::pollfd,
                events.sources.list.len() as libc::nfds_t,
                timeout,
            )
        };
        events.count = result as usize;

        if result == 0 {
            if self.is_empty() {
                Ok(())
            } else {
                Err(io::ErrorKind::TimedOut.into())
            }
        } else if result > 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
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
    ///     use popol::{Sources, Events, Waker};
    ///
    ///     const WAKER: &'static str = "waker";
    ///
    ///     let mut events = Events::new();
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
    ///     sources.wait(&mut events, Duration::from_secs(1))?;
    ///
    ///     assert!(!events.is_empty(), "There should be at least one event selected");
    ///
    ///     let mut events = events.iter();
    ///     let (key, event) = events.next().unwrap();
    ///
    ///     assert!(key == &WAKER, "The event is triggered by the waker");
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

        sources.insert(key, Source::new(fd, interest::READ));

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

        let mut events = Events::new();
        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1, &reader2] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, interest::READ);
        sources.register("reader1", &reader1, interest::READ);
        sources.register("reader2", &reader2, interest::READ);

        {
            let err = sources
                .wait(&mut events, Duration::from_millis(1))
                .unwrap_err();

            assert_eq!(err.kind(), io::ErrorKind::TimedOut);
            assert!(events.is_empty());
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

            sources.wait(&mut events, Duration::from_millis(1))?;
            assert!(!events.is_empty());

            let mut events = events.iter();
            let (k, event) = events.next().unwrap();

            assert_eq!(&k, &key);
            assert!(event.readable && !event.writable && !event.errored && !event.hangup);
            assert!(events.next().is_none());

            assert_eq!(reader.read(&mut buf[..])?, 1);
            assert_eq!(&buf[..], &[*byte]);
        }
        Ok(())
    }

    #[test]
    fn test_empty() -> io::Result<()> {
        let mut events: Events<()> = Events::new();
        let mut sources = Sources::new();

        sources
            .wait(&mut events, time::Duration::from_millis(1))
            .expect("no error if nothing registered");

        assert!(events.is_empty());

        Ok(())
    }

    #[test]
    fn test_timeout() -> io::Result<()> {
        let mut events = Events::new();
        let mut sources = Sources::new();

        sources.register((), &io::stdin(), interest::READ);

        let err = sources
            .wait(&mut events, Duration::from_millis(1))
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

        let mut events = Events::new();
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
                writer.write(&[1]).unwrap();
                writer.write(&[2]).unwrap();
            }
        });

        let mut closed = vec![];
        while closed.len() < readers.len() {
            sources.wait(&mut events, Duration::from_millis(64))?;

            for (key, event) in events.iter() {
                assert!(event.readable);
                assert!(!event.writable);
                assert!(!event.errored);

                if event.hangup {
                    closed.push(key.clone());
                    continue;
                }

                let mut buf = [0u8; 2];
                let mut reader = match key {
                    &"reader0" => &reader0,
                    &"reader1" => &reader1,
                    &"reader2" => &reader2,
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

        let mut events = Events::new();
        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1, &reader2] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, interest::READ);
        sources.register("reader1", &reader1, interest::READ);
        sources.register("reader2", &reader2, interest::READ);

        {
            let err = sources
                .wait(&mut events, Duration::from_millis(1))
                .unwrap_err();

            assert_eq!(err.kind(), io::ErrorKind::TimedOut);
            assert!(events.is_empty());
        }

        {
            writer1.write(&[0x0])?;

            sources.wait(&mut events, Duration::from_millis(1))?;
            let (key, _) = events.iter().next().unwrap();

            assert_eq!(key, &"reader1");
        }

        // Unregister.
        {
            sources.unregister(&"reader1");
            writer1.write(&[0x0])?;

            sources.wait(&mut events, Duration::from_millis(1)).ok();
            assert!(events.iter().next().is_none());

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write(&[0])?;
            }

            sources.wait(&mut events, Duration::from_millis(1))?;
            let keys = events.iter().map(|(k, _)| k).collect::<HashSet<_>>();

            assert!(keys.contains(&"reader0"));
            assert!(!keys.contains(&"reader1"));
            assert!(keys.contains(&"reader2"));

            sources.unregister(&"reader0");

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write(&[0])?;
            }

            sources.wait(&mut events, Duration::from_millis(1))?;
            let keys = events.iter().map(|(k, _)| k).collect::<HashSet<_>>();

            assert!(!keys.contains(&"reader0"));
            assert!(!keys.contains(&"reader1"));
            assert!(keys.contains(&"reader2"));

            sources.unregister(&"reader2");

            for w in &mut [&writer0, &writer1, &writer2] {
                w.write(&[0])?;
            }

            sources.wait(&mut events, Duration::from_millis(1)).ok();

            assert!(events.is_empty());
        }

        // Re-register.
        {
            sources.register("reader0", &reader0, interest::READ);
            writer0.write(&[0])?;

            sources.wait(&mut events, Duration::from_millis(1))?;
            let (key, _) = events.iter().next().unwrap();

            assert_eq!(key, &"reader0");
        }

        Ok(())
    }

    #[test]
    fn test_set() -> io::Result<()> {
        let (mut writer0, reader0) = UnixStream::pair()?;
        let (mut writer1, reader1) = UnixStream::pair()?;

        let mut events = Events::new();
        let mut sources = Sources::new();

        for reader in &[&reader0, &reader1] {
            reader.set_nonblocking(true)?;
        }

        sources.register("reader0", &reader0, interest::READ);
        sources.register("reader1", &reader1, interest::NONE);

        {
            writer0.write(&[0])?;

            sources.wait(&mut events, Duration::from_millis(1))?;
            let (key, _) = events.iter().next().unwrap();
            assert_eq!(key, &"reader0");

            sources.unset(key, interest::READ);
            writer0.write(&[0])?;

            sources.wait(&mut events, Duration::from_millis(1)).ok();
            assert!(events.iter().next().is_none());
        }

        {
            writer1.write(&[0])?;

            sources.wait(&mut events, Duration::from_millis(1)).ok();
            assert!(events.iter().next().is_none());

            sources.set(&"reader1", interest::READ);
            writer1.write(&[0])?;

            sources.wait(&mut events, Duration::from_millis(1))?;
            let (key, _) = events.iter().next().unwrap();
            assert_eq!(key, &"reader1");
        }

        Ok(())
    }

    #[test]
    fn test_waker() -> io::Result<()> {
        let mut events = Events::new();
        let mut sources = Sources::new();
        let mut waker = Waker::new(&mut sources, "waker")?;
        let buf = [0; 4096];

        sources.wait(&mut events, Duration::from_millis(1)).ok();
        assert!(events.iter().next().is_none());

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

        sources.wait(&mut events, Duration::from_millis(1))?;
        let (key, event) = events.iter().next().unwrap();

        assert!(event.readable);
        assert!(!event.writable && !event.hangup && !event.errored);
        assert_eq!(key, &"waker");

        waker.wake()?;

        sources.wait(&mut events, Duration::from_millis(1))?;
        let (key, event) = events.iter().next().unwrap();

        assert!(event.readable);
        assert_eq!(key, &"waker");

        // Try to wake multiple times.
        waker.wake()?;
        waker.wake()?;
        waker.wake()?;

        sources.wait(&mut events, Duration::from_millis(1))?;
        assert_eq!(events.iter().count(), 1, "multiple wakes count as one");

        Ok(())
    }
}
