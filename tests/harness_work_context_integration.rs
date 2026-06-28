use prometheos_lite::harness::*;

#[test]
fn harness_public_surface_is_available() {
    let limits = HarnessLimits::default();
    assert!(limits.max_steps > 0);
    let confidence = ConfidenceScore {
        score: 0.8,
        factors: vec![ConfidenceFactor {
            name: "test".into(),
            weight: 1.0,
            score: 0.8,
            description: "test factor".into(),
            impact: FactorImpact::Positive,
        }],
        explanation: "Test confidence".into(),
        recommendation: None,
    };
    assert!(confidence.score > 0.0);
}
