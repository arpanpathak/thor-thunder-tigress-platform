//! How many model requests each person may have running at once, and how many
//! they may start in a minute.
//!
//! An engine answers one request at a time, so one person's script could hold
//! it for everyone. The operator, who holds the single key, has no limits.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

/// Who holds the single key.
pub const OPERATOR: &str = "operator";

/// The span the per-minute count covers.
const MINUTE: Duration = Duration::from_secs(60);

/// The limits, and what each person is using now.
pub struct Limits {
    per_minute: usize,
    at_once: usize,
    usage: Mutex<HashMap<String, Usage>>,
}

/// One person's running requests and the start times within the last minute.
#[derive(Default)]
struct Usage {
    running: usize,
    started: VecDeque<Instant>,
}

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The person already has as many requests running as allowed.
    #[error("too many requests running at once")]
    Busy,
    /// The person started as many requests as allowed in the last minute.
    #[error("too many requests this minute; retry in {retry_after} s")]
    Spent {
        /// Seconds until the oldest start is a minute old.
        retry_after: u64,
    },
}

impl Refusal {
    /// Seconds until a new request may be admitted.
    #[must_use]
    pub fn retry_after(self) -> u64 {
        match self {
            Refusal::Busy => 1,
            Refusal::Spent { retry_after } => retry_after,
        }
    }
}

/// A running request's place in its person's limits, given back when dropped.
pub struct Permit {
    who: Option<String>,
    limits: Arc<Limits>,
}

impl Limits {
    /// Limits of `per_minute` requests a minute and `at_once` at the same time.
    #[must_use]
    pub fn new(per_minute: usize, at_once: usize) -> Arc<Self> {
        Arc::new(Limits {
            per_minute,
            at_once,
            usage: Mutex::new(HashMap::new()),
        })
    }

    /// Admits a request from `who` at `now`, or says when to try again.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] when `who` already has `at_once` requests running or has
    /// started `per_minute` in the last minute.
    pub fn admit(self: &Arc<Self>, who: &str, now: Instant) -> Result<Permit, Refusal> {
        if who == OPERATOR {
            return Ok(Permit {
                who: None,
                limits: Arc::clone(self),
            });
        }

        self.admit_person(who, now)
    }

    fn admit_person(self: &Arc<Self>, who: &str, now: Instant) -> Result<Permit, Refusal> {
        let mut usage = self.lock();
        let person = usage.entry(who.to_string()).or_default();
        forget_before(person, now);

        if let Some(refusal) = self.busy(person).or_else(|| self.spent(person, now)) {
            return Err(refusal);
        }

        person.running += 1;
        person.started.push_back(now);

        Ok(Permit {
            who: Some(who.to_string()),
            limits: Arc::clone(self),
        })
    }

    /// How many requests `who` has running.
    #[cfg(test)]
    #[must_use]
    pub fn running(&self, who: &str) -> usize {
        self.lock().get(who).map_or(0, |person| person.running)
    }

    fn busy(&self, person: &Usage) -> Option<Refusal> {
        (person.running >= self.at_once).then_some(Refusal::Busy)
    }

    fn spent(&self, person: &Usage, now: Instant) -> Option<Refusal> {
        let oldest = person
            .started
            .front()
            .filter(|_| person.started.len() >= self.per_minute)?;
        let waited = now.saturating_duration_since(*oldest);

        Some(Refusal::Spent {
            retry_after: MINUTE.saturating_sub(waited).as_secs().max(1),
        })
    }

    fn release(&self, who: &str) {
        let mut usage = self.lock();
        let running = usage.get_mut(who).map(|person| &mut person.running);

        if let Some(running) = running {
            *running = running.saturating_sub(1);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Usage>> {
        self.usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Drops the start times older than a minute before `now`.
fn forget_before(person: &mut Usage, now: Instant) {
    while person
        .started
        .front()
        .is_some_and(|start| now.saturating_duration_since(*start) >= MINUTE)
    {
        person.started.pop_front();
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(who) = &self.who {
            self.limits.release(who);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_up_to_the_running_limit_and_again_after_a_release() {
        let limits = Limits::new(100, 2);
        let now = Instant::now();

        let first = limits.admit("ada", now);
        let second = limits.admit("ada", now);
        let third = limits.admit("ada", now);

        assert!(first.is_ok() && second.is_ok());
        assert_eq!(third.err(), Some(Refusal::Busy));
        assert_eq!(Refusal::Busy.retry_after(), 1);
        assert_eq!(limits.running("ada"), 2);

        drop(first);

        assert_eq!(limits.running("ada"), 1);
        assert!(limits.admit("ada", now).is_ok());
    }

    #[test]
    fn refuses_past_the_minute_limit_until_the_oldest_start_is_a_minute_old() {
        let limits = Limits::new(2, 10);
        let start = Instant::now();

        drop(limits.admit("ada", start));
        drop(limits.admit("ada", start + Duration::from_secs(20)));
        let refused = limits.admit("ada", start + Duration::from_secs(45));

        assert_eq!(refused.map(|_| ()).map_err(Refusal::retry_after), Err(15));
        assert!(limits.admit("ada", start + MINUTE).is_ok());
    }

    #[test]
    fn counts_each_person_apart_and_never_limits_the_operator() {
        let limits = Limits::new(1, 1);
        let now = Instant::now();

        let ada = limits.admit("ada", now);
        let grace = limits.admit("grace", now);
        let operator: Vec<_> = (0..5).map(|_| limits.admit(OPERATOR, now)).collect();

        assert!(ada.is_ok() && grace.is_ok());
        assert!(operator.iter().all(Result::is_ok));
        assert_eq!(limits.running(OPERATOR), 0);
    }

    #[test]
    fn a_release_for_someone_unknown_changes_nothing() {
        let limits = Limits::new(1, 1);

        limits.release("nobody");

        assert_eq!(limits.running("nobody"), 0);
    }
}
