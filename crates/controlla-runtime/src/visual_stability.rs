use sha2::Digest;
use std::{future::Future, pin::Pin, time::Duration};

/// Decode Chrome's bounded screenshot and normalize it to tightly packed RGBA8.
/// The returned digest is independent of PNG compression and ancillary chunks.
pub fn normalized_pixel_digest(
    png_bytes: &[u8],
) -> Result<(String, VisualRegion), VisualStabilityError> {
    if png_bytes.len() < 57
        || png_bytes.len() > 6 * 1024 * 1024
        || png_bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
    {
        return Err(VisualStabilityError {
            reason_code: "invalid_visual_stability_sample",
            message: "Chrome screenshot is empty, oversized, or not a PNG",
        });
    }
    let width = u32::from_be_bytes(png_bytes[16..20].try_into().unwrap_or_default());
    let height = u32::from_be_bytes(png_bytes[20..24].try_into().unwrap_or_default());
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > 16_777_216
    {
        return Err(VisualStabilityError {
            reason_code: "invalid_visual_stability_sample",
            message: "Chrome screenshot dimensions exceed visual stability bounds",
        });
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| VisualStabilityError {
        reason_code: "invalid_visual_stability_sample",
        message: "Chrome returned an invalid PNG frame",
    })?;
    let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|_| VisualStabilityError {
            reason_code: "invalid_visual_stability_sample",
            message: "Chrome returned an invalid PNG frame",
        })?;
    if info.width == 0
        || info.height == 0
        || info.width > 4096
        || info.height > 4096
        || u64::from(info.width) * u64::from(info.height) > 16_777_216
    {
        return Err(VisualStabilityError {
            reason_code: "invalid_visual_stability_sample",
            message: "Chrome screenshot dimensions exceed visual stability bounds",
        });
    }
    let pixels = &decoded[..info.buffer_size()];
    let mut rgba = Vec::with_capacity((info.width as usize) * (info.height as usize) * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(pixels),
        png::ColorType::Rgb => {
            for rgb in pixels.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for gray in pixels {
                rgba.extend_from_slice(&[*gray, *gray, *gray, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for gray_alpha in pixels.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[1],
                ]);
            }
        }
        png::ColorType::Indexed => {
            return Err(VisualStabilityError {
                reason_code: "invalid_visual_stability_sample",
                message: "PNG palette was not expanded",
            });
        }
    }
    let digest = sha2::Sha256::digest(&rgba);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok((
        hex,
        VisualRegion {
            x: 0,
            y: 0,
            width: info.width,
            height: info.height,
        },
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualIdentity {
    pub session_id: String,
    pub tab_id: String,
    pub document_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualStabilityConfig {
    pub region: Option<VisualRegion>,
    pub quiet_ms: u64,
    pub timeout_ms: u64,
    pub poll_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualStabilitySample {
    pub identity: VisualIdentity,
    pub target_active: bool,
    /// SHA-256 over normalized RGBA pixels for the requested region.
    pub pixel_digest: String,
    pub visual_revision: String,
    pub geometry: VisualRegion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualStabilityResult {
    pub stable: bool,
    pub reason_code: Option<String>,
    pub elapsed_ms: u64,
    pub samples: u32,
    pub visual_revision: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualStabilityError {
    pub reason_code: &'static str,
    pub message: &'static str,
}

pub type FrameFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VisualStabilitySample, VisualStabilityError>> + Send + 'a>>;

// The strictest supported configuration is a 10s timeout with 50ms polling.
// Include the initial capture and every interval so the public bounds are real.
const MAX_VISUAL_STABILITY_SAMPLES: u32 = 201;

pub trait VisualFrameSource: Send {
    fn capture<'a>(
        &'a mut self,
        identity: &'a VisualIdentity,
        region: Option<VisualRegion>,
    ) -> FrameFuture<'a>;
}

pub async fn wait_for_visual_stability<S: VisualFrameSource>(
    source: &mut S,
    identity: &VisualIdentity,
    config: VisualStabilityConfig,
) -> Result<VisualStabilityResult, VisualStabilityError> {
    validate_visual_stability_config(config)?;
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_millis(config.timeout_ms);
    wait_for_visual_stability_until(source, identity, config, started, deadline).await
}

pub async fn wait_for_visual_stability_until<S: VisualFrameSource>(
    source: &mut S,
    identity: &VisualIdentity,
    config: VisualStabilityConfig,
    started: tokio::time::Instant,
    deadline: tokio::time::Instant,
) -> Result<VisualStabilityResult, VisualStabilityError> {
    validate_visual_stability_config(config)?;
    let mut samples = 0u32;
    let mut prior_digest: Option<String> = None;
    let mut quiet_since = started;
    let mut visual_revision = None;

    loop {
        if tokio::time::Instant::now() >= deadline || samples >= MAX_VISUAL_STABILITY_SAMPLES {
            return Ok(VisualStabilityResult {
                stable: false,
                reason_code: Some("visual_stability_timeout".into()),
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                samples,
                visual_revision,
            });
        }
        let sample = match tokio::time::timeout_at(
            deadline,
            source.capture(identity, config.region),
        )
        .await
        {
            Err(_) => {
                return Ok(VisualStabilityResult {
                    stable: false,
                    reason_code: Some("visual_stability_timeout".into()),
                    elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                    samples,
                    visual_revision,
                });
            }
            Ok(Err(error))
                if matches!(
                    error.reason_code,
                    "stale_visual_reference" | "inactive_target" | "visual_stability_timeout"
                ) =>
            {
                return Ok(VisualStabilityResult {
                    stable: false,
                    reason_code: Some(error.reason_code.into()),
                    elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                    samples: samples.saturating_add(1),
                    visual_revision: None,
                });
            }
            Ok(Err(error)) => return Err(error),
            Ok(Ok(sample)) => sample,
        };
        samples = samples.saturating_add(1);
        if sample.identity != *identity {
            return Ok(VisualStabilityResult {
                stable: false,
                reason_code: Some("stale_visual_reference".into()),
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                samples,
                visual_revision: None,
            });
        }
        if !sample.target_active {
            return Ok(VisualStabilityResult {
                stable: false,
                reason_code: Some("inactive_target".into()),
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                samples,
                visual_revision: None,
            });
        }
        if !region_fits(config.region, sample.geometry) {
            return Err(VisualStabilityError {
                reason_code: "invalid_visual_stability_bounds",
                message: "visual stability region exceeds current screenshot geometry",
            });
        }
        if sample.pixel_digest.is_empty() || sample.visual_revision.is_empty() {
            return Err(VisualStabilityError {
                reason_code: "invalid_visual_stability_sample",
                message: "visual stability samples require a pixel digest and revision",
            });
        }
        if prior_digest.as_deref() != Some(sample.pixel_digest.as_str()) {
            prior_digest = Some(sample.pixel_digest);
            quiet_since = tokio::time::Instant::now();
        }
        visual_revision = Some(sample.visual_revision);
        if quiet_since.elapsed() >= Duration::from_millis(config.quiet_ms) {
            return Ok(VisualStabilityResult {
                stable: true,
                reason_code: None,
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                samples,
                visual_revision,
            });
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(config.poll_ms)).min(deadline),
        )
        .await;
    }
}

pub fn validate_visual_stability_config(
    config: VisualStabilityConfig,
) -> Result<(), VisualStabilityError> {
    let valid_region = config.region.is_none_or(|region| {
        region.width > 0
            && region.height > 0
            && region
                .x
                .checked_add(region.width)
                .is_some_and(|right| right <= 4096)
            && region
                .y
                .checked_add(region.height)
                .is_some_and(|bottom| bottom <= 4096)
            && u64::from(region.width) * u64::from(region.height) <= 16_777_216
    });
    if !(150..=2_000).contains(&config.quiet_ms)
        || config.timeout_ms < config.quiet_ms
        || config.timeout_ms > 10_000
        || !(50..=500).contains(&config.poll_ms)
        || !valid_region
    {
        return Err(VisualStabilityError {
            reason_code: "invalid_visual_stability_bounds",
            message: "visual stability region or timing is outside supported bounds",
        });
    }
    Ok(())
}

fn region_fits(region: Option<VisualRegion>, geometry: VisualRegion) -> bool {
    let Some(region) = region else { return true };
    region
        .x
        .checked_add(region.width)
        .is_some_and(|right| right <= geometry.width)
        && region
            .y
            .checked_add(region.height)
            .is_some_and(|bottom| bottom <= geometry.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn identity() -> VisualIdentity {
        VisualIdentity {
            session_id: "session-1".into(),
            tab_id: "tab-1".into(),
            document_id: "doc-1".into(),
        }
    }

    fn sample(id: &VisualIdentity, digest: &str, active: bool) -> VisualStabilitySample {
        VisualStabilitySample {
            identity: id.clone(),
            target_active: active,
            pixel_digest: digest.into(),
            visual_revision: format!("sha256:{digest}"),
            geometry: VisualRegion {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
        }
    }

    struct Frames(VecDeque<VisualStabilitySample>);

    impl VisualFrameSource for Frames {
        fn capture<'a>(
            &'a mut self,
            _identity: &'a VisualIdentity,
            _region: Option<VisualRegion>,
        ) -> FrameFuture<'a> {
            Box::pin(async move { Ok(self.0.pop_front().expect("fixture frame")) })
        }
    }

    fn config() -> VisualStabilityConfig {
        VisualStabilityConfig {
            region: Some(VisualRegion {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            }),
            quiet_ms: 150,
            timeout_ms: 500,
            poll_ms: 50,
        }
    }

    #[tokio::test]
    async fn visual_stability_requires_same_document_identity() {
        let id = identity();
        let mut changed = id.clone();
        changed.document_id = "doc-2".into();
        let mut frames = Frames(VecDeque::from([
            sample(&id, "a", true),
            sample(&changed, "a", true),
        ]));
        let result = wait_for_visual_stability(&mut frames, &id, config())
            .await
            .unwrap();
        assert!(!result.stable);
        assert_eq!(
            result.reason_code.as_deref(),
            Some("stale_visual_reference")
        );
    }

    #[tokio::test]
    async fn visual_stability_requires_active_target_lifecycle() {
        let id = identity();
        let mut frames = Frames(VecDeque::from([sample(&id, "a", false)]));
        let result = wait_for_visual_stability(&mut frames, &id, config())
            .await
            .unwrap();
        assert!(!result.stable);
        assert_eq!(result.reason_code.as_deref(), Some("inactive_target"));
    }

    #[tokio::test]
    async fn visual_stability_returns_stable_after_quiet_interval() {
        let id = identity();
        let mut frames = Frames(VecDeque::from([
            sample(&id, "a", true),
            sample(&id, "b", true),
            sample(&id, "b", true),
            sample(&id, "b", true),
            sample(&id, "b", true),
            sample(&id, "b", true),
        ]));
        let result = wait_for_visual_stability(&mut frames, &id, config())
            .await
            .unwrap();
        assert!(result.stable);
        assert_eq!(result.visual_revision.as_deref(), Some("sha256:b"));
        assert!(result.samples >= 4);
    }

    #[tokio::test]
    async fn visual_stability_times_out_without_claiming_stable() {
        let id = identity();
        let mut frames = Frames(VecDeque::from([
            sample(&id, "a", true),
            sample(&id, "b", true),
            sample(&id, "c", true),
            sample(&id, "d", true),
            sample(&id, "e", true),
            sample(&id, "f", true),
            sample(&id, "g", true),
            sample(&id, "h", true),
            sample(&id, "i", true),
            sample(&id, "j", true),
            sample(&id, "k", true),
            sample(&id, "l", true),
        ]));
        let mut cfg = config();
        cfg.timeout_ms = 300;
        let result = wait_for_visual_stability(&mut frames, &id, cfg)
            .await
            .unwrap();
        assert!(!result.stable);
        assert_eq!(
            result.reason_code.as_deref(),
            Some("visual_stability_timeout")
        );
    }

    #[tokio::test]
    async fn visual_stability_uses_the_caller_deadline_and_elapsed_origin() {
        let id = identity();
        let mut frames = Frames(VecDeque::from([
            sample(&id, "a", true),
            sample(&id, "a", true),
            sample(&id, "a", true),
        ]));
        let started = tokio::time::Instant::now() - Duration::from_millis(300);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(30);
        let result = wait_for_visual_stability_until(&mut frames, &id, config(), started, deadline)
            .await
            .unwrap();
        assert!(!result.stable);
        assert_eq!(
            result.reason_code.as_deref(),
            Some("visual_stability_timeout")
        );
        assert!(result.elapsed_ms >= 300);
    }

    #[tokio::test]
    async fn visual_stability_rejects_unbounded_region_and_timeout() {
        let id = identity();
        let mut frames = Frames(VecDeque::from([sample(&id, "a", true)]));
        let mut cfg = config();
        cfg.region = Some(VisualRegion {
            x: 400,
            y: 0,
            width: 1,
            height: 1,
        });
        let result = wait_for_visual_stability(&mut frames, &id, cfg)
            .await
            .unwrap_err();
        assert_eq!(result.reason_code, "invalid_visual_stability_bounds");

        let mut cfg = config();
        cfg.timeout_ms = 10_001;
        let result = wait_for_visual_stability(&mut frames, &id, cfg)
            .await
            .unwrap_err();
        assert_eq!(result.reason_code, "invalid_visual_stability_bounds");
    }

    #[test]
    fn normalized_pixel_digest_ignores_png_encoding_and_color_model() {
        fn png(color: png::ColorType, data: &[u8]) -> Vec<u8> {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
                encoder.set_color(color);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(data).unwrap();
            }
            bytes
        }
        let rgb = png(png::ColorType::Rgb, &[12, 34, 56]);
        let rgba = png(png::ColorType::Rgba, &[12, 34, 56, 255]);
        let (rgb_hash, _) = normalized_pixel_digest(&rgb).unwrap();
        let (rgba_hash, _) = normalized_pixel_digest(&rgba).unwrap();
        assert_eq!(rgb_hash, rgba_hash);
    }
}
