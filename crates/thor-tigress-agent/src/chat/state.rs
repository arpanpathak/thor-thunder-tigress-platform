//! The search loop's states, and the turns that move it from one to the next.
//!
//! ```text
//!   Researching { round }
//!     ├─ Again(Calls), rounds and searches left ─────────► Researching { round + 1 }
//!     ├─ Again(Calls) with none left, or Again(Nothing) ─► Answering { round: 1, AsAsked }
//!     ├─ Again(Stuck) ───────────────────────────────────► Answering { round: 1, Off }
//!     └─ Answer or Cut ──────────────────────────────────► Done
//!
//!   Answering { round, think }
//!     ├─ Again(Stuck), thinking as asked, rounds left ───► Answering { round + 1, Off }
//!     ├─ Again(Calls or Nothing), rounds left ───────────► Answering { round + 1, think }
//!     └─ Answer, Cut, stuck again, or the last round ────► Done
//! ```
//!
//! Every transition here is a plain function of its inputs, so the tests can
//! check each one and drive every turn until the loop ends.

use super::{ANSWER_ROUNDS, MAX_ROUNDS, wire::Round};

/// The answer text, in bytes, past which a round has written the answer and
/// not a line before a search. Every round's text reaches the page as it
/// streams, so a round after it would write the whole answer again.
pub(super) const ANSWERED_CHARS: usize = 1024;

/// Where a searched answer stands between two rounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    /// The tools are offered. `round` counts from one up to [`MAX_ROUNDS`].
    Researching {
        /// The research round about to be asked for, counting from one.
        round: usize,
    },
    /// The tools are gone and the model is asked for the answer.
    Answering {
        /// The answering round about to be asked for, counting from one up to
        /// [`ANSWER_ROUNDS`].
        round: usize,
        /// Whether this round may think first.
        think: Think,
    },
    /// The answer is over, in the way named.
    Done(End),
}

/// Whether an answering round may think first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Think {
    /// As the request asked.
    AsAsked,
    /// Off, because a round's thinking was stopped by the repeat guard.
    Off,
}

/// How an answer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    /// The model wrote it.
    Answered,
    /// The guard stopped a round, for the reason held; the page is told it.
    Stopped(&'static str),
    /// No round wrote anything, so the page gets the note naming the sources.
    Unanswered,
}

/// What one round gave back, read once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Turn {
    /// The answer: text with no call, text of [`ANSWERED_CHARS`] or more, or
    /// text whose calls all ran already. Its text is on the page, so another
    /// round would write it a second time.
    Answer,
    /// The guard stopped the round, for the reason held, after answer text
    /// had reached the page.
    Cut(&'static str),
    /// No answer yet: the loop asks again, if it still may.
    Again(Again),
}

/// Why a round leaves the loop to ask again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Again {
    /// Calls to run, perhaps after a short line saying what they look up.
    Calls,
    /// Neither text nor a call.
    Nothing,
    /// The guard stopped the round, for the reason held, before any answer
    /// text: thinking that looped. Nothing reached the page, so the round
    /// can be asked again.
    Stuck(&'static str),
}

/// How much answer text a round wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Written {
    /// None, or only whitespace.
    Nothing,
    /// Less than [`ANSWERED_CHARS`]: perhaps a line before a search.
    Short,
    /// [`ANSWERED_CHARS`] or more: the answer.
    Long,
}

/// Which calls a round made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Called {
    /// None.
    Nothing,
    /// At least one call that has not run yet in this answer.
    New,
    /// Only calls that already ran in this answer.
    AlreadyRan,
}

impl Written {
    /// How much answer text `content` holds, leaving out the whitespace
    /// around it.
    fn of(content: &str) -> Written {
        match content.trim().len() {
            0 => Written::Nothing,
            1..ANSWERED_CHARS => Written::Short,
            ANSWERED_CHARS.. => Written::Long,
        }
    }
}

impl Called {
    /// Which calls a round made. `already_ran` says whether every one of
    /// them ran in an earlier round.
    fn of(round: &Round, already_ran: bool) -> Called {
        match (round.calls.is_empty(), already_ran) {
            (true, _) => Called::Nothing,
            (false, true) => Called::AlreadyRan,
            (false, false) => Called::New,
        }
    }
}

impl Turn {
    /// Reads a round. `already_ran` says whether every call it made ran in
    /// an earlier round.
    pub(super) fn of(round: &Round, already_ran: bool) -> Turn {
        let written = Written::of(&round.content);
        let called = Called::of(round, already_ran);

        match (round.stop, written, called) {
            (Some(reason), Written::Nothing, _) => Turn::Again(Again::Stuck(reason)),
            (Some(reason), Written::Short | Written::Long, _) => Turn::Cut(reason),
            (None, Written::Nothing, Called::Nothing) => Turn::Again(Again::Nothing),
            (None, Written::Nothing, Called::New | Called::AlreadyRan)
            | (None, Written::Short, Called::New) => Turn::Again(Again::Calls),
            (None, Written::Short, Called::Nothing | Called::AlreadyRan)
            | (None, Written::Long, _) => Turn::Answer,
        }
    }
}

impl State {
    /// The first answering round, with thinking as `think` says.
    pub(super) const fn answering(think: Think) -> State {
        State::Answering { round: 1, think }
    }

    /// The state after research round `round` gave back `turn`. Research
    /// goes on while calls come back and rounds and searches are left; stuck
    /// thinking moves to the answer with thinking off.
    pub(super) const fn after_research(round: usize, turn: Turn, out_of_searches: bool) -> State {
        match (turn, another_research_round(round, out_of_searches)) {
            (Turn::Answer, _) => State::Done(End::Answered),
            (Turn::Cut(reason), _) => State::Done(End::Stopped(reason)),
            (Turn::Again(Again::Calls), true) => State::Researching { round: round + 1 },
            (Turn::Again(Again::Calls), false) => State::answering(Think::AsAsked),
            (Turn::Again(Again::Nothing), _) => State::answering(Think::AsAsked),
            (Turn::Again(Again::Stuck(_)), _) => State::answering(Think::Off),
        }
    }

    /// The state after answering round `round`, asked with `think`, gave back
    /// `turn`. Stuck thinking is asked once more with thinking off; stuck
    /// again, the answer stops. A round with calls or nothing uses up a round.
    pub(super) const fn after_answer(round: usize, think: Think, turn: Turn) -> State {
        let more = round < ANSWER_ROUNDS;

        match (turn, think, more) {
            (Turn::Answer, _, _) => State::Done(End::Answered),
            (Turn::Cut(reason), _, _) => State::Done(End::Stopped(reason)),
            (Turn::Again(Again::Stuck(_)), Think::AsAsked, true) => State::Answering {
                round: round + 1,
                think: Think::Off,
            },
            (Turn::Again(Again::Stuck(reason)), Think::Off, _)
            | (Turn::Again(Again::Stuck(reason)), Think::AsAsked, false) => {
                State::Done(End::Stopped(reason))
            }
            (Turn::Again(Again::Calls | Again::Nothing), _, true) => State::Answering {
                round: round + 1,
                think,
            },
            (Turn::Again(Again::Calls | Again::Nothing), _, false) => State::Done(End::Unanswered),
        }
    }
}

/// Whether research round `round` may be followed by another: rounds are
/// left and so are searches.
const fn another_research_round(round: usize, out_of_searches: bool) -> bool {
    round < MAX_ROUNDS && !out_of_searches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::wire::ToolCall;

    const REASON: &str = "Stopped because the model kept repeating the same text.";
    const CALLS: Turn = Turn::Again(Again::Calls);
    const NOTHING: Turn = Turn::Again(Again::Nothing);
    const STUCK: Turn = Turn::Again(Again::Stuck(REASON));
    const STOPPED: State = State::Done(End::Stopped(REASON));
    const ANSWERED: State = State::Done(End::Answered);

    /// Research transitions: the round, what it gave back, whether the
    /// searches ran out, and the state that follows.
    #[rustfmt::skip]
    const RESEARCH: [(usize, Turn, bool, State); 8] = [
        (1, CALLS, false, State::Researching { round: 2 }),
        (MAX_ROUNDS, CALLS, false, State::answering(Think::AsAsked)),
        (1, CALLS, true, State::answering(Think::AsAsked)),
        (1, NOTHING, false, State::answering(Think::AsAsked)),
        (1, STUCK, false, State::answering(Think::Off)),
        (MAX_ROUNDS, STUCK, false, State::answering(Think::Off)),
        (1, Turn::Answer, false, ANSWERED),
        (1, Turn::Cut(REASON), false, STOPPED),
    ];

    /// Answering transitions: the round, its thinking, what it gave back,
    /// and the state that follows.
    #[rustfmt::skip]
    const ANSWERING: [(usize, Think, Turn, State); 9] = [
        (1, Think::AsAsked, STUCK, State::Answering { round: 2, think: Think::Off }),
        (1, Think::Off, STUCK, STOPPED),
        (ANSWER_ROUNDS, Think::AsAsked, STUCK, STOPPED),
        (1, Think::AsAsked, NOTHING, State::Answering { round: 2, think: Think::AsAsked }),
        (2, Think::Off, CALLS, State::Answering { round: 3, think: Think::Off }),
        (ANSWER_ROUNDS, Think::AsAsked, CALLS, State::Done(End::Unanswered)),
        (ANSWER_ROUNDS, Think::Off, NOTHING, State::Done(End::Unanswered)),
        (1, Think::AsAsked, Turn::Answer, ANSWERED),
        (ANSWER_ROUNDS, Think::Off, Turn::Cut(REASON), STOPPED),
    ];

    #[test]
    fn every_research_round_moves_to_the_state_the_table_names() {
        for (round, turn, out_of_searches, next) in RESEARCH {
            assert_eq!(
                State::after_research(round, turn, out_of_searches),
                next,
                "round {round} after {turn:?}"
            );
        }
    }

    #[test]
    fn every_answering_round_moves_to_the_state_the_table_names() {
        for (round, think, turn, next) in ANSWERING {
            assert_eq!(
                State::after_answer(round, think, turn),
                next,
                "round {round} {think:?} after {turn:?}"
            );
        }
    }

    /// The state after `state` gave back `turn`, with searches left.
    fn after(state: State, turn: Turn) -> State {
        match state {
            State::Researching { round } => State::after_research(round, turn, false),
            State::Answering { round, think } => State::after_answer(round, think, turn),
            State::Done(end) => State::Done(end),
        }
    }

    /// How many rounds the loop asks for when every round gives back
    /// `turn`, counted up to one past the most it may ever ask for.
    fn rounds_until_done(turn: Turn) -> usize {
        let start = State::Researching { round: 1 };

        std::iter::successors(Some(start), |state| Some(after(*state, turn)))
            .take(MAX_ROUNDS + ANSWER_ROUNDS + 1)
            .take_while(|state| !matches!(state, State::Done(_)))
            .count()
    }

    #[test]
    fn the_loop_ends_whatever_the_model_keeps_doing() {
        assert_eq!(rounds_until_done(CALLS), MAX_ROUNDS + ANSWER_ROUNDS);
        assert_eq!(rounds_until_done(NOTHING), 1 + ANSWER_ROUNDS);
        assert_eq!(rounds_until_done(STUCK), 2);
    }

    /// A round that wrote `content`, made `calls` calls and was stopped for
    /// `stop`, when it was.
    fn round(content: &str, calls: usize, stop: Option<&'static str>) -> Round {
        Round {
            content: content.to_string(),
            calls: vec![ToolCall::default(); calls],
            stop,
        }
    }

    #[test]
    fn a_round_is_read_as_the_turn_its_text_calls_and_stop_make() {
        let long = "a".repeat(ANSWERED_CHARS);
        let cases = [
            (round("", 0, Some(REASON)), false, STUCK),
            (round(" \n", 1, Some(REASON)), false, STUCK),
            (
                round("half an answer", 0, Some(REASON)),
                false,
                Turn::Cut(REASON),
            ),
            (round("", 0, None), true, NOTHING),
            (round("", 1, None), false, CALLS),
            (round("", 1, None), true, CALLS),
            (round("Looking up the jobs.", 1, None), false, CALLS),
            (round("The answer.", 0, None), true, Turn::Answer),
            (round("The answer.", 1, None), true, Turn::Answer),
            (round(&long, 1, None), false, Turn::Answer),
            (round(&long, 0, None), true, Turn::Answer),
        ];

        for (reply, already_ran, turn) in cases {
            assert_eq!(Turn::of(&reply, already_ran), turn, "{reply:?}");
        }
    }
}
