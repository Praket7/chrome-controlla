use controlla_runtime::visual_stability::{
    FrameFuture, VisualFrameSource, VisualIdentity, VisualRegion, VisualStabilityConfig,
    VisualStabilitySample, wait_for_visual_stability,
};
use std::time::Duration;

struct FixtureFrames {
    identity: VisualIdentity,
    digest: &'static str,
    delay: Duration,
    calls: u32,
    requested_region: Option<VisualRegion>,
}

impl VisualFrameSource for FixtureFrames {
    fn capture<'a>(
        &'a mut self,
        _identity: &'a VisualIdentity,
        region: Option<VisualRegion>,
    ) -> FrameFuture<'a> {
        Box::pin(async move {
            self.calls += 1;
            self.requested_region = region;
            tokio::time::sleep(self.delay).await;
            Ok(VisualStabilitySample {
                identity: self.identity.clone(),
                target_active: true,
                pixel_digest: self.digest.into(),
                visual_revision: format!("sha256:{}", self.digest),
                geometry: VisualRegion {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
            })
        })
    }
}

fn identity() -> VisualIdentity {
    VisualIdentity {
        session_id: "benchmark".into(),
        tab_id: "tab-1".into(),
        document_id: "doc-1".into(),
    }
}

fn percentile(sorted: &[u128], percent: usize) -> u128 {
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

#[tokio::test]
async fn visual_stability_fixture_benchmark_covers_ten_warm_and_ten_cold_runs() {
    let id = identity();
    let mut measurements = Vec::new();
    for (class, delay) in [
        ("warm", Duration::from_millis(1)),
        ("cold", Duration::from_millis(5)),
    ] {
        let mut durations = Vec::new();
        for _ in 0..10 {
            let mut source = FixtureFrames {
                identity: id.clone(),
                digest: "same-region-pixels",
                delay,
                calls: 0,
                requested_region: None,
            };
            let started = std::time::Instant::now();
            let result = wait_for_visual_stability(
                &mut source,
                &id,
                VisualStabilityConfig {
                    region: Some(VisualRegion {
                        x: 20,
                        y: 30,
                        width: 300,
                        height: 200,
                    }),
                    quiet_ms: 150,
                    timeout_ms: 800,
                    poll_ms: 50,
                },
            )
            .await
            .unwrap();
            assert!(result.stable);
            assert_eq!(
                source.requested_region,
                Some(VisualRegion {
                    x: 20,
                    y: 30,
                    width: 300,
                    height: 200
                })
            );
            assert!(source.calls <= 6);
            durations.push(started.elapsed().as_millis());
        }
        durations.sort_unstable();
        measurements.push((
            class,
            percentile(&durations, 50),
            percentile(&durations, 95),
        ));
    }
    eprintln!("synthetic visual fixture benchmark (not Chrome timing): {measurements:?}");
    assert_eq!(measurements.len(), 2);
}

#[tokio::test]
async fn visual_fixture_source_can_model_outside_region_changes_without_a_false_instability() {
    let id = identity();
    let region = VisualRegion {
        x: 20,
        y: 30,
        width: 300,
        height: 200,
    };
    let mut source = FixtureFrames {
        identity: id.clone(),
        digest: "stable-selected-region",
        delay: Duration::ZERO,
        calls: 0,
        requested_region: None,
    };
    let result = wait_for_visual_stability(
        &mut source,
        &id,
        VisualStabilityConfig {
            region: Some(region),
            quiet_ms: 150,
            timeout_ms: 500,
            poll_ms: 50,
        },
    )
    .await
    .unwrap();
    assert!(result.stable);
    assert_eq!(source.requested_region, Some(region));
}
