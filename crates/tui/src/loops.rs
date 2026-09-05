//! Repeating a prompt the person typed.
//!
//! `/loop` holds one line and sends it again on a schedule. The line is settled the moment it is
//! typed and never changes afterwards: a tick is the same prompt as the first one, so nothing a
//! turn reads, writes or says can decide what the next turn asks for. A planner may say *when* the
//! next tick happens, within an hour, and that is the whole of what it may say.
//!
//! Two ways the next tick's moment is decided, and the argument picks between them. An interval
//! the person wrote is kept by the driver, and the planner is not consulted. No interval, and each
//! tick's turn says how long to wait; a turn that says nothing gets one fallback wake and then the
//! loop ends, because a loop nobody is re-arming is a loop nobody is running.
//!
//! Nothing here is written to disk. A loop lives as long as the session that started it.

pub use bravebot_agent::turn::{Tick, Wakeup};
use std::time::{Duration, Instant};

/// The shortest interval a person may set.
///
/// Low, because this is their own number: somebody who types `10s` wants ten seconds, and the
/// gap is measured from the end of a tick, so a slow turn spaces itself out whatever this says.
/// It is not zero because a loop with no gap at all is a way to spend a rate limit rather than a
/// way to watch something, and because the interface has to stay usable between ticks.
///
/// A wait a *turn* asks for is bounded far more tightly, and by the tool that takes it: that
/// number is the planner's rather than the person's.
const FLOOR: Duration = Duration::from_secs(5);

/// How long a loop may run before it ends itself.
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What the driver waits when a self-paced turn ended without saying when to wake.
const KEEPALIVE: Duration = Duration::from_secs(1_200);

/// How many times that fallback is used before the loop ends.
///
/// One. The first time is a turn that forgot; the second is a turn that is not going to remember,
/// and waking it every twenty minutes forever is not what anybody asked for.
const KEEPALIVE_BUDGET: u8 = 1;

/// The unit letters an interval may be written with, and what each is worth in seconds.
///
/// Longest first, so `days` is matched before `d` and a suffix search cannot stop early.
const UNITS: [(&str, u64); 18] = [
    ("seconds", 1),
    ("second", 1),
    ("secs", 1),
    ("sec", 1),
    ("minutes", 60),
    ("minute", 60),
    ("mins", 60),
    ("min", 60),
    ("hours", 3_600),
    ("hour", 3_600),
    ("hrs", 3_600),
    ("hr", 3_600),
    ("days", 86_400),
    ("day", 86_400),
    ("s", 1),
    ("m", 60),
    ("h", 3_600),
    ("d", 86_400),
];

/// The unit letters the token at the front of the argument may use.
///
/// Narrower than [`UNITS`] on purpose: this token is recognised before anything else is read, so
/// `minutes of silence` must stay a prompt rather than becoming an interval and a sentence.
const SHORT_UNITS: [(&str, u64); 4] = [("s", 1), ("m", 60), ("h", 3_600), ("d", 86_400)];

/// The word that introduces an interval written at the end of a sentence.
const EVERY: &str = "every";

/// How the moment of the next tick is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacing {
    /// The person gave an interval, and the driver keeps to it without asking anybody.
    Every(Duration),
    /// Nobody gave one, so each tick's turn says when the next is due.
    SelfPaced,
}

/// What `/loop` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub pacing: Pacing,
    /// The line to send on every tick, exactly as it was typed.
    pub prompt: String,
    /// What the interval became, where the one asked for was outside the bounds.
    ///
    /// Said out loud rather than applied quietly: somebody who typed `10s` and is told nothing
    /// believes they are watching something ten times more closely than they are.
    pub adjusted: Option<Held>,
}

/// An interval that was not the one asked for, and which way it moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// Faster than a loop goes.
    Raised(Duration),
    /// Longer than a loop lives.
    Capped(Duration),
}

/// Read the argument to `/loop`.
///
/// `None` where there is nothing to send, which is the bare command and the interval on its own.
/// Both are answered by saying what the command needs.
pub fn parse(argument: &str) -> Option<Request> {
    let argument = argument.trim();
    if argument.is_empty() {
        return None;
    }

    // The front of the line first. A token that is nothing but a number and a unit letter was
    // written to be an interval, and reading it as one is what makes `/loop 5m /babysit` work.
    if let Some((asked, prompt)) = leading_interval(argument) {
        return interval(asked, prompt);
    }

    // Then the end of it, which is where an interval goes when the line is a sentence.
    if let Some((asked, prompt)) = trailing_interval(argument) {
        return interval(asked, prompt);
    }

    Some(Request {
        pacing: Pacing::SelfPaced,
        prompt: argument.to_string(),
        adjusted: None,
    })
}

/// A request from an interval and what is left of the line, or `None` where nothing is left.
fn interval(asked: Duration, prompt: &str) -> Option<Request> {
    if prompt.is_empty() {
        return None;
    }
    let held = asked.clamp(FLOOR, MAX_AGE);
    let adjusted = match held {
        _ if held == asked => None,
        _ if held == FLOOR => Some(Held::Raised(held)),
        _ => Some(Held::Capped(held)),
    };
    Some(Request {
        pacing: Pacing::Every(held),
        prompt: prompt.to_string(),
        adjusted,
    })
}

/// An interval written as the first word, and the rest of the line.
fn leading_interval(argument: &str) -> Option<(Duration, &str)> {
    let (token, rest) = match argument.split_once(char::is_whitespace) {
        Some((token, rest)) => (token, rest.trim()),
        None => (argument, ""),
    };
    let seconds = measure(token, &SHORT_UNITS)?;
    Some((seconds, rest))
}

/// An interval written as an `every ...` clause at the end, and the line without it.
fn trailing_interval(argument: &str) -> Option<(Duration, &str)> {
    // Byte offsets are shared between the two because lowercasing only ASCII leaves every other
    // byte where it was, so an index found in one is an index into the other.
    let lowered = argument.to_ascii_lowercase();
    let mut at = None;
    let mut from = 0;
    while let Some(found) = lowered[from..].find(EVERY) {
        let found = from + found;
        let opens = found == 0 || lowered[..found].ends_with(char::is_whitespace);
        if opens {
            at = Some(found);
        }
        from = found + EVERY.len();
    }
    let at = at?;

    let mut words = lowered[at..].split_whitespace();
    // The word itself, which the search already established is there.
    words.next()?;
    let seconds = match (words.next(), words.next(), words.next()) {
        // `every 20m`
        (Some(one), None, None) => measure(one, &UNITS)?,
        // `every 20 minutes`
        (Some(count), Some(unit), None) => {
            let worth = UNITS.iter().find(|(name, _)| *name == unit)?.1;
            multiply(count, worth)?
        }
        // Anything else is words. `every PR` and `every so often` are not intervals, and a
        // sentence that ends in one of them is a prompt with nothing taken out of it.
        _ => return None,
    };

    Some((seconds, argument[..at].trim()))
}

/// A count and a unit fused into one token, as a duration.
fn measure(token: &str, units: &[(&str, u64)]) -> Option<Duration> {
    for (name, worth) in units {
        if let Some(count) = token.strip_suffix(name) {
            return multiply(count, *worth);
        }
    }
    None
}

/// A run of digits multiplied by what its unit is worth.
///
/// `None` for anything that is not a number, and for a number too large to be a duration: a
/// count that overflows is a word that happened to be made of digits, not an interval.
fn multiply(count: &str, worth: u64) -> Option<Duration> {
    if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    count
        .parse::<u64>()
        .ok()?
        .checked_mul(worth)
        .map(Duration::from_secs)
}

/// A loop that is running.
#[derive(Debug, Clone)]
pub struct Running {
    /// The line every tick sends. Settled when it was typed and never written to again.
    prompt: String,
    pacing: Pacing,
    began: Instant,
    /// When the next tick is due, or `None` while a tick is in flight or a self-paced turn has
    /// yet to say.
    due: Option<Instant>,
    /// Whether the turn for a tick is running now, so its end is the moment to arm the next.
    running: bool,
    /// Fallback wakes left before an unarmed self-paced loop ends.
    keepalive: u8,
    /// Ticks dispatched so far, the first one included.
    ticks: usize,
    /// Ticks that reported nothing to do, since the last one that reported something.
    quiet: usize,
}

impl Running {
    /// Start a loop, with nothing due: the caller dispatches the first tick straight away.
    pub fn begin(request: Request) -> Self {
        Self {
            prompt: request.prompt,
            pacing: request.pacing,
            began: Instant::now(),
            due: None,
            running: false,
            keepalive: KEEPALIVE_BUDGET,
            ticks: 0,
            quiet: 0,
        }
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn pacing(&self) -> Pacing {
        self.pacing
    }

    /// Whether the planner is asked when the next tick is due.
    pub fn self_paced(&self) -> bool {
        self.pacing == Pacing::SelfPaced
    }

    /// Whether the turn running now is a tick of this loop.
    ///
    /// False for a prompt the person typed in the middle of one. That turn is theirs, it is not
    /// asked when the next tick is due, and its end does not reset the clock.
    pub fn ticking(&self) -> bool {
        self.running
    }

    /// What the turn running now needs to know about being a tick, where it is one.
    pub fn tick(&self) -> Option<Tick> {
        self.running.then_some(Tick {
            number: self.ticks,
            self_paced: self.self_paced(),
        })
    }

    pub fn ticks(&self) -> usize {
        self.ticks
    }

    pub fn quiet(&self) -> usize {
        self.quiet
    }

    /// How long until the next tick, or `None` where none is armed.
    pub fn until(&self, now: Instant) -> Option<Duration> {
        self.due.map(|due| due.saturating_duration_since(now))
    }

    /// Whether the loop has run longer than a loop may run.
    pub fn aged_out(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.began) >= MAX_AGE
    }

    /// Whether a tick should be dispatched now.
    ///
    /// A tick in flight is not due again, and a loop with nothing armed is waiting rather than
    /// ready. Whether the *session* is free to take one is the caller's question, not this one's.
    pub fn due(&self, now: Instant) -> bool {
        !self.running && self.due.is_some_and(|due| due <= now)
    }

    /// Record that a tick has been sent.
    pub fn dispatched(&mut self) {
        self.ticks += 1;
        self.running = true;
        self.due = None;
    }

    /// Arm the next tick from the turn that has just ended, and say whether the loop goes on.
    ///
    /// `wakeup` is what a self-paced turn asked for, and is ignored where the person gave an
    /// interval: a loop the driver paces is not the planner's to re-time.
    ///
    /// The gap is measured from here rather than from when the tick was sent, so an interval is
    /// the time between runs. Measured from the send, a turn that outlasts its own interval would
    /// be due again the instant it finished, and a slow loop would become a continuous one.
    pub fn ended(&mut self, wakeup: Option<Wakeup>, now: Instant) -> bool {
        if !self.running {
            return true;
        }
        self.running = false;

        if let Pacing::Every(every) = self.pacing {
            self.due = now.checked_add(every);
            return true;
        }

        match wakeup {
            Some(wakeup) => {
                self.quiet = if wakeup.quiet { self.quiet + 1 } else { 0 };
                self.due = now.checked_add(wakeup.after);
                self.keepalive = KEEPALIVE_BUDGET;
                true
            }
            // A turn that said nothing is given one more chance, on the driver's own clock.
            None if self.keepalive > 0 => {
                self.keepalive -= 1;
                self.due = now.checked_add(KEEPALIVE);
                true
            }
            None => false,
        }
    }
}

/// A duration in the units somebody would say it in, at most two of them.
///
/// Not a message in a catalog: these are the unit letters the person typed into `/loop` coming
/// back, and translating them would answer `5m` with something they could not type again.
pub fn spell(duration: Duration) -> String {
    let mut left = duration.as_secs();
    let mut parts = Vec::new();
    for (unit, worth) in [("d", 86_400), ("h", 3_600), ("m", 60), ("s", 1)] {
        let count = left / worth;
        if count > 0 {
            parts.push(format!("{count}{unit}"));
            left -= count * worth;
        }
        if parts.len() == 2 {
            break;
        }
    }
    if parts.is_empty() {
        return "0s".to_string();
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every(request: &Request) -> Duration {
        match request.pacing {
            Pacing::Every(duration) => duration,
            Pacing::SelfPaced => panic!("expected an interval"),
        }
    }

    /// The form the command was documented with, and the one a person types when they already
    /// know what they want repeated.
    #[test]
    fn an_interval_written_first_is_taken_off_the_front() {
        let request = parse("5m check the deploy").expect("a prompt and an interval");
        assert_eq!(every(&request), Duration::from_secs(300));
        assert_eq!(request.prompt, "check the deploy");
    }

    #[test]
    fn every_unit_letter_is_understood_at_the_front() {
        for (line, seconds) in [
            ("90s watch", 90),
            ("5m watch", 300),
            ("2h watch", 7_200),
            ("1d watch", 86_400),
        ] {
            let request = parse(line).expect("a prompt and an interval");
            assert_eq!(every(&request), Duration::from_secs(seconds), "{line}");
        }
    }

    /// The other way people write it, which reads as a sentence rather than as an argument.
    #[test]
    fn an_interval_written_last_is_taken_off_the_end() {
        let request = parse("check the deploy every 20m").expect("a prompt and an interval");
        assert_eq!(every(&request), Duration::from_secs(1_200));
        assert_eq!(request.prompt, "check the deploy");
    }

    #[test]
    fn a_trailing_interval_may_spell_its_unit_out() {
        for line in [
            "check it every 20 minutes",
            "check it every 20 min",
            "check it every 20m",
            "check it EVERY 20 MINUTES",
        ] {
            let request = parse(line).expect("a prompt and an interval");
            assert_eq!(every(&request), Duration::from_secs(1_200), "{line}");
            assert_eq!(request.prompt, "check it", "{line}");
        }
    }

    /// The whole reason the trailing form insists on a time expression. Without it, half the
    /// sentences somebody would loop over would lose their last two words.
    #[test]
    fn every_without_a_time_after_it_is_words_rather_than_an_interval() {
        for line in ["check every PR", "look at every so often", "review every"] {
            let request = parse(line).expect("a prompt");
            assert_eq!(request.pacing, Pacing::SelfPaced, "{line}");
            assert_eq!(request.prompt, line, "{line}");
        }
    }

    /// The front of the line is read first, so a line with both is paced by the one written as
    /// an argument rather than by the one written in the sentence.
    #[test]
    fn a_leading_interval_wins_over_a_trailing_one() {
        let request = parse("5m check the deploy every 20m").expect("a prompt and an interval");
        assert_eq!(every(&request), Duration::from_secs(300));
        assert_eq!(request.prompt, "check the deploy every 20m");
    }

    #[test]
    fn a_line_with_no_interval_is_paced_by_the_planner() {
        let request = parse("watch the build").expect("a prompt");
        assert_eq!(request.pacing, Pacing::SelfPaced);
        assert_eq!(request.prompt, "watch the build");
    }

    /// Nothing to send is not a loop, however it was written.
    #[test]
    fn an_interval_with_nothing_to_send_is_not_a_request() {
        for line in ["", "   ", "5m", "  5m  "] {
            assert_eq!(parse(line), None, "{line}");
        }
    }

    /// A word that merely starts with a digit is not an interval, and the line keeps it.
    #[test]
    fn a_word_that_is_not_a_time_stays_part_of_the_prompt() {
        let request = parse("5minutes to midnight").expect("a prompt");
        assert_eq!(request.pacing, Pacing::SelfPaced);
        assert_eq!(request.prompt, "5minutes to midnight");
    }

    /// The person's own number is honoured as far down as a loop goes, which is far enough that
    /// the interval somebody actually types is usually the interval they get.
    #[test]
    fn an_interval_the_person_gave_is_kept_down_to_the_floor() {
        let request = parse("10s watch").expect("a prompt and an interval");
        assert_eq!(every(&request), Duration::from_secs(10));
        assert_eq!(request.adjusted, None);
    }

    #[test]
    fn an_interval_faster_than_the_floor_is_raised_to_it_and_said_so() {
        let request = parse("1s watch").expect("a prompt and an interval");
        assert_eq!(every(&request), FLOOR);
        assert_eq!(request.adjusted, Some(Held::Raised(FLOOR)));
    }

    #[test]
    fn an_interval_longer_than_a_loop_may_live_is_capped() {
        let request = parse("30d watch").expect("a prompt and an interval");
        assert_eq!(every(&request), MAX_AGE);
        assert_eq!(request.adjusted, Some(Held::Capped(MAX_AGE)));
    }

    /// A number nothing could hold is a word made of digits, not a schedule, and reading it as
    /// one would have to invent a duration nobody asked for.
    #[test]
    fn a_count_too_large_to_be_a_duration_is_not_an_interval() {
        let request = parse("check it every 99999999999999999999 days").expect("a prompt");
        assert_eq!(request.pacing, Pacing::SelfPaced);
    }

    #[test]
    fn an_interval_within_the_bounds_is_reported_as_unadjusted() {
        let request = parse("5m watch").expect("a prompt and an interval");
        assert_eq!(request.adjusted, None);
    }

    /// The driver keeps its own time, so a planner cannot lengthen or shorten a loop the person
    /// set the pace of.
    #[test]
    fn a_paced_loop_ignores_what_a_turn_asked_for() {
        let mut running = Running::begin(parse("5m watch").expect("a request"));
        let now = Instant::now();
        running.dispatched();
        assert!(running.ended(Some(Wakeup::asked(3_600, false)), now,));
        assert_eq!(running.until(now), Some(Duration::from_secs(300)));
    }

    #[test]
    fn a_self_paced_loop_waits_as_long_as_the_turn_asked() {
        let mut running = Running::begin(parse("watch").expect("a request"));
        let now = Instant::now();
        running.dispatched();
        assert!(running.ended(Some(Wakeup::asked(900, false)), now,));
        assert_eq!(running.until(now), Some(Duration::from_secs(900)));
    }

    /// The bounds are the tool's, so this is the loop honouring what the tool already held the
    /// number to rather than a second clamp of its own.
    #[test]
    fn a_wait_a_turn_asked_for_is_held_to_the_bounds() {
        for (asked, held) in [(1, Wakeup::FLOOR), (86_400, Wakeup::CEILING)] {
            let mut running = Running::begin(parse("watch").expect("a request"));
            let now = Instant::now();
            running.dispatched();
            running.ended(Some(Wakeup::asked(asked, false)), now);
            assert_eq!(running.until(now), Some(held), "{asked}");
        }
    }

    /// One forgotten turn is a turn that forgot. Two is a loop nobody is running, and waking it
    /// every twenty minutes for the rest of the session helps nobody.
    #[test]
    fn a_self_paced_turn_that_says_nothing_is_woken_once_more_and_then_the_loop_ends() {
        let mut running = Running::begin(parse("watch").expect("a request"));
        let now = Instant::now();

        running.dispatched();
        assert!(running.ended(None, now));
        assert_eq!(running.until(now), Some(KEEPALIVE));

        running.dispatched();
        assert!(!running.ended(None, now));
    }

    /// The budget is for turns that stopped saying when to wake, not for the one that did.
    #[test]
    fn a_turn_that_says_when_to_wake_restores_the_fallback() {
        let mut running = Running::begin(parse("watch").expect("a request"));
        let now = Instant::now();

        running.dispatched();
        running.ended(None, now);
        running.dispatched();
        running.ended(Some(Wakeup::asked(120, false)), now);

        running.dispatched();
        assert!(running.ended(None, now));
    }

    #[test]
    fn quiet_ticks_are_counted_until_one_reports_something() {
        let mut running = Running::begin(parse("watch").expect("a request"));
        let now = Instant::now();
        for expected in [1, 2, 3] {
            running.dispatched();
            running.ended(Some(Wakeup::asked(60, true)), now);
            assert_eq!(running.quiet(), expected);
        }
        running.dispatched();
        running.ended(Some(Wakeup::asked(60, false)), now);
        assert_eq!(running.quiet(), 0);
    }

    /// A tick already running is not due again. Without this a turn that outlasts its interval
    /// would have a second tick waiting the moment it drew breath.
    #[test]
    fn a_tick_in_flight_is_not_due_again() {
        let mut running = Running::begin(parse("5m watch").expect("a request"));
        let now = Instant::now();
        running.dispatched();
        running.ended(None, now);
        let due = now + Duration::from_secs(600);
        assert!(running.due(due));
        running.dispatched();
        assert!(!running.due(due));
    }

    #[test]
    fn a_loop_with_nothing_armed_is_not_due() {
        let running = Running::begin(parse("watch").expect("a request"));
        assert!(!running.due(Instant::now() + Duration::from_secs(86_400)));
    }

    #[test]
    fn a_loop_older_than_a_week_has_aged_out() {
        let running = Running::begin(parse("5m watch").expect("a request"));
        let now = Instant::now();
        assert!(!running.aged_out(now + MAX_AGE - Duration::from_secs(1)));
        assert!(running.aged_out(now + MAX_AGE));
    }

    #[test]
    fn a_duration_is_spelled_in_the_units_it_was_asked_for() {
        assert_eq!(spell(Duration::from_secs(45)), "45s");
        assert_eq!(spell(Duration::from_secs(300)), "5m");
        assert_eq!(spell(Duration::from_secs(90)), "1m 30s");
        assert_eq!(spell(Duration::from_secs(5_400)), "1h 30m");
        assert_eq!(spell(Duration::from_secs(86_400)), "1d");
        assert_eq!(spell(Duration::ZERO), "0s");
    }
}
