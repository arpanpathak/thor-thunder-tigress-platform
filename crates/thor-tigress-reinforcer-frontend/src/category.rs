//! The words a flag is made of: which slop category, and which part of an
//! example.

use serde::{Deserialize, Serialize};
use thor_spark_safety_eval::slop::Category;

/// The slop categories of `anti_ai_slop.md`, plus `other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlopCategory {
    /// "This is where it really matters."
    FakeImportance,
    /// "Here's the thing:" before something ordinary.
    DramaticSetup,
    /// "Delve into the rich tapestry."
    EmptyDepthWords,
    /// "It's worth noting both have merits."
    FakeBalanceHedging,
    /// "Great question!"
    FlatteryFillerOpener,
    /// "In summary," repeating the answer.
    WrapUpRepeat,
    /// Triplets, one-word fragments, em-dash reveals.
    RhythmTrick,
    /// Anything else, such as markup that leaked into an answer.
    Other,
}

/// The part of an example a phrase was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    /// What was asked.
    Instruction,
    /// What was answered.
    Response,
}

impl SlopCategory {
    /// Every category, in the order the picker offers them.
    pub const ALL: [SlopCategory; 8] = [
        SlopCategory::FakeImportance,
        SlopCategory::DramaticSetup,
        SlopCategory::EmptyDepthWords,
        SlopCategory::FakeBalanceHedging,
        SlopCategory::FlatteryFillerOpener,
        SlopCategory::WrapUpRepeat,
        SlopCategory::RhythmTrick,
        SlopCategory::Other,
    ];

    /// The name used in the flag files.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SlopCategory::FakeImportance => "fake_importance",
            SlopCategory::DramaticSetup => "dramatic_setup",
            SlopCategory::EmptyDepthWords => "empty_depth_words",
            SlopCategory::FakeBalanceHedging => "fake_balance_hedging",
            SlopCategory::FlatteryFillerOpener => "flattery_filler_opener",
            SlopCategory::WrapUpRepeat => "wrap_up_repeat",
            SlopCategory::RhythmTrick => "rhythm_trick",
            SlopCategory::Other => "other",
        }
    }

    /// The category in words, for the picker.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            SlopCategory::FakeImportance => "fake importance",
            SlopCategory::DramaticSetup => "dramatic setup before something ordinary",
            SlopCategory::EmptyDepthWords => "empty depth words",
            SlopCategory::FakeBalanceHedging => "fake balance and hedging",
            SlopCategory::FlatteryFillerOpener => "flattery and filler openers",
            SlopCategory::WrapUpRepeat => "wrap-ups that repeat the answer",
            SlopCategory::RhythmTrick => "rhythm tricks",
            SlopCategory::Other => "other",
        }
    }
}

impl Field {
    /// The name used in the flag files.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Field::Instruction => "instruction",
            Field::Response => "response",
        }
    }
}

impl From<Category> for SlopCategory {
    /// The flag category of a Stage 0 (`spark`) slop category.
    fn from(category: Category) -> Self {
        match category {
            Category::FakeImportance => SlopCategory::FakeImportance,
            Category::DramaticSetup => SlopCategory::DramaticSetup,
            Category::EmptyDepthWords => SlopCategory::EmptyDepthWords,
            Category::FakeBalanceHedging => SlopCategory::FakeBalanceHedging,
            Category::FlatteryFillerOpener => SlopCategory::FlatteryFillerOpener,
            Category::WrapUpRepeat => SlopCategory::WrapUpRepeat,
            Category::RhythmTrick => SlopCategory::RhythmTrick,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Outcome;

    fn written(value: &impl serde::Serialize) -> Outcome<String> {
        serde_json::to_string(value).map_err(|error| crate::error::ReviewError::BadRequest(error.to_string()))
    }

    #[test]
    fn names_are_what_the_flag_files_hold() -> Outcome {
        let names = SlopCategory::ALL.map(SlopCategory::name);
        assert_eq!(
            names,
            [
                "fake_importance", "dramatic_setup", "empty_depth_words", "fake_balance_hedging",
                "flattery_filler_opener", "wrap_up_repeat", "rhythm_trick", "other",
            ]
        );
        for category in SlopCategory::ALL {
            assert_eq!(written(&category)?, format!("\"{}\"", category.name()));
        }
        for field in [Field::Instruction, Field::Response] {
            assert_eq!(written(&field)?, format!("\"{}\"", field.name()));
        }
        Ok(())
    }

    #[test]
    fn unknown_names_are_refused() {
        assert!(serde_json::from_str::<SlopCategory>(r#""made_up""#).is_err());
        assert!(serde_json::from_str::<Field>(r#""title""#).is_err());
    }

    #[test]
    fn every_category_has_words_and_stage0_maps_onto_it() {
        assert!(SlopCategory::ALL.iter().all(|category| !category.description().is_empty()));
        assert_eq!(SlopCategory::from(Category::WrapUpRepeat), SlopCategory::WrapUpRepeat);
        assert_eq!(SlopCategory::from(Category::FakeImportance), SlopCategory::FakeImportance);
    }
}
