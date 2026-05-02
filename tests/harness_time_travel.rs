use prometheos_lite::harness::*;

#[test]
fn harness_public_surface_is_available() {
    let limits = HarnessLimits::default();
    assert!(limits.max_steps > 0);
    let confidence = ConfidenceScore {
        score: 0.8,
        factors: vec!["test".to_string()],
    };
    assert!(confidence.score > 0.0);
}
