//! Explicit evaluation state-transition law.
//!
//! The pipeline is a bounded state machine. Every transition must be declared
//! here and validated by [`validate_transition`] before the new state is
//! applied to the journal and/or identity document. This makes illegal jumps
//! (for example, jumping straight from `Created` to `ReviewGate`) a hard
//! error rather than a silent corruption.

use anyhow::{Result, bail};

use super::identity::EvaluationState;

/// Whether a transition from `from` to `to` is allowed.
///
/// This encodes the happy path, the recoverable resume path, and the terminal
/// failure entry points. Anything not listed here is rejected as an illegal
/// transition.
pub fn validate_transition(from: EvaluationState, to: EvaluationState) -> Result<()> {
    // A terminal state is final; no transition out of it is ever legal.
    if from.is_terminal() {
        bail!("illegal transition from terminal state {from:?} to {to:?}");
    }
    // Identical, non-terminal state is an idempotent no-op (allowed for resume).
    if from == to {
        return Ok(());
    }
    let legal = matches!(
        (from, to),
        // Happy path.
        (EvaluationState::Created, EvaluationState::PreflightPassed)
            | (EvaluationState::PreflightPassed, EvaluationState::Generating)
            | (EvaluationState::Generating, EvaluationState::ProposalGenerated)
            | (EvaluationState::ProposalGenerated, EvaluationState::GovernancePassed)
            | (EvaluationState::GovernancePassed, EvaluationState::Validating)
            | (EvaluationState::Validating, EvaluationState::ValidationComplete)
            | (EvaluationState::ValidationComplete, EvaluationState::IntegrityVerified)
            | (EvaluationState::IntegrityVerified, EvaluationState::ReviewGate)
            // Resume path: generation is done, validation may start directly.
            | (EvaluationState::ProposalGenerated, EvaluationState::Validating)
            // Terminal failure entry points (from the immediately preceding stage).
            | (EvaluationState::Created, EvaluationState::PreflightBlocked)
            | (EvaluationState::PreflightPassed, EvaluationState::GenerationFailed)
            | (EvaluationState::Generating, EvaluationState::GenerationFailed)
            | (EvaluationState::ProposalGenerated, EvaluationState::GovernanceRejected)
            | (EvaluationState::GovernancePassed, EvaluationState::ValidationFailed)
            | (EvaluationState::Validating, EvaluationState::ValidationFailed)
            | (EvaluationState::Validating, EvaluationState::InfraBlocked)
            | (EvaluationState::ValidationComplete, EvaluationState::IntegrityFailed)
            | (EvaluationState::ProposalGenerated, EvaluationState::CandidateCompileFailed)
            | (EvaluationState::GovernancePassed, EvaluationState::CandidateTestFailed)
            // Terminal outcome entry points applied at finalization.
            | (EvaluationState::IntegrityVerified, EvaluationState::IntegrityFailed)
            | (EvaluationState::IntegrityVerified, EvaluationState::ValidationFailed)
            | (EvaluationState::IntegrityVerified, EvaluationState::CandidateCompileFailed)
            | (EvaluationState::IntegrityVerified, EvaluationState::CandidateTestFailed)
            | (EvaluationState::IntegrityVerified, EvaluationState::InfraBlocked)
    );
    if legal {
        Ok(())
    } else {
        bail!("illegal evaluation state transition: {from:?} -> {to:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_chain_is_legal() {
        let chain = [
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            EvaluationState::Generating,
            EvaluationState::ProposalGenerated,
            EvaluationState::GovernancePassed,
            EvaluationState::Validating,
            EvaluationState::ValidationComplete,
            EvaluationState::IntegrityVerified,
            EvaluationState::ReviewGate,
        ];
        for pair in chain.windows(2) {
            assert!(
                validate_transition(pair[0], pair[1]).is_ok(),
                "legal transition rejected: {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn rejects_illegal_jumps() {
        assert!(
            validate_transition(EvaluationState::Created, EvaluationState::ReviewGate).is_err()
        );
        assert!(
            validate_transition(EvaluationState::Created, EvaluationState::Validating).is_err()
        );
        assert!(
            validate_transition(
                EvaluationState::Generating,
                EvaluationState::IntegrityVerified
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_successive_transitions() {
        // Releasing a terminal failure then progressing is illegal.
        assert!(
            validate_transition(EvaluationState::PreflightBlocked, EvaluationState::Created)
                .is_err()
        );
        assert!(
            validate_transition(
                EvaluationState::ReviewGate,
                EvaluationState::IntegrityVerified
            )
            .is_err()
        );
        assert!(
            validate_transition(
                EvaluationState::ValidationFailed,
                EvaluationState::Validating
            )
            .is_err()
        );
    }

    #[test]
    fn terminal_failure_entry_points_are_allowed() {
        assert!(
            validate_transition(EvaluationState::Created, EvaluationState::PreflightBlocked)
                .is_ok()
        );
        assert!(
            validate_transition(
                EvaluationState::Generating,
                EvaluationState::GenerationFailed
            )
            .is_ok()
        );
        assert!(
            validate_transition(
                EvaluationState::ProposalGenerated,
                EvaluationState::GovernanceRejected
            )
            .is_ok()
        );
        assert!(
            validate_transition(
                EvaluationState::Validating,
                EvaluationState::ValidationFailed
            )
            .is_ok()
        );
        assert!(
            validate_transition(EvaluationState::Validating, EvaluationState::InfraBlocked).is_ok()
        );
    }

    #[test]
    fn idempotent_same_state_is_allowed() {
        // Resume/retry may re-assert the current non-terminal state.
        assert!(
            validate_transition(EvaluationState::Validating, EvaluationState::Validating).is_ok()
        );
        assert!(validate_transition(EvaluationState::Created, EvaluationState::Created).is_ok());
    }
}
