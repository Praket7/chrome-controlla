//! Offline CapCut Web recipe validation. This compiles caller-supplied media metadata and timing
//! into a bounded plan; it does not resolve controls or execute anything in CapCut.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ASSETS: usize = 5;
const MAX_CAPTIONS: usize = 3;
const MAX_ID_LEN: usize = 80;
const MAX_TEXT_BYTES: usize = 500;
const MIN_AUDIO_GAIN_MILLIDB: i32 = -60_000;
const MAX_AUDIO_GAIN_MILLIDB: i32 = 6_000;

#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    rmcp::schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AssetUse {
    ShadeShot,
    WaterRestShot,
    NeighborShot,
    Narration,
    Music,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct LicensedAsset {
    pub asset_id: String,
    pub source_and_rights: String,
    pub sha256: String,
    pub duration_ms: u64,
    pub frame_rate_milli: u32,
    pub width: u32,
    pub height: u32,
    pub audio_tracks: u8,
    pub expected_use: AssetUse,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct VideoClip {
    pub asset_id: String,
    pub source_in_ms: u64,
    pub source_out_ms: u64,
    pub timeline_start_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct Caption {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct AudioTrack {
    pub asset_id: String,
    pub source_in_ms: u64,
    pub source_out_ms: u64,
    pub timeline_start_ms: u64,
    /// Gain expressed in milli-decibels; this is a plan value, not app evidence.
    pub gain_millidb: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct EndCard {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub background_hex: String,
    pub text_hex: String,
    pub font_size_px: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CapCutRecipe {
    pub assets: Vec<LicensedAsset>,
    pub clips: Vec<VideoClip>,
    pub captions: Vec<Caption>,
    pub narration: AudioTrack,
    pub music: AudioTrack,
    pub end_card: EndCard,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct TimelineClipPlan {
    pub asset_id: String,
    pub source_in_ms: u64,
    pub source_out_ms: u64,
    pub timeline_start_ms: u64,
    pub timeline_end_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CapCutTimelinePlan {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub clips: Vec<TimelineClipPlan>,
    pub captions: Vec<Caption>,
    pub narration: AudioTrack,
    pub music: AudioTrack,
    pub end_card: EndCard,
    /// Preparation output only. This must never be treated as app execution evidence.
    pub qualification: String,
}

/// Validates the fixed Phase 8 brief and returns a serializable timeline description.
/// All license, checksum, clip, and caption timing values come from the caller.
pub fn compile_capcut_recipe(recipe: &CapCutRecipe) -> Result<CapCutTimelinePlan, &'static str> {
    if recipe.assets.len() != MAX_ASSETS {
        return Err("recipe requires exactly three shots, narration, and music assets");
    }
    if recipe.width != 1920 || recipe.height != 1080 {
        return Err("recipe output must be 1920x1080 (16:9)");
    }

    let mut assets = BTreeMap::new();
    for asset in &recipe.assets {
        if !valid_id(&asset.asset_id)
            || asset.source_and_rights.trim().is_empty()
            || asset.source_and_rights.len() > MAX_TEXT_BYTES
            || !valid_sha256(&asset.sha256)
            || asset.duration_ms == 0
            || asset.frame_rate_milli == 0
            || asset.width == 0
            || asset.height == 0
        {
            return Err("asset metadata, rights, checksum, or media dimensions are invalid");
        }
        if assets.insert(asset.asset_id.as_str(), asset).is_some() {
            return Err("asset IDs must be unique");
        }
    }

    let expected = [
        AssetUse::ShadeShot,
        AssetUse::WaterRestShot,
        AssetUse::NeighborShot,
        AssetUse::Narration,
        AssetUse::Music,
    ];
    let uses: BTreeSet<_> = recipe
        .assets
        .iter()
        .map(|asset| asset.expected_use)
        .collect();
    if uses.len() != expected.len() || expected.iter().any(|kind| !uses.contains(kind)) {
        return Err("assets must include each required shot, narration, and music role once");
    }
    let narration_asset = assets
        .get(recipe.narration.asset_id.as_str())
        .filter(|asset| asset.expected_use == AssetUse::Narration && asset.audio_tracks > 0)
        .ok_or("narration ID must identify an asset with an audio track")?;
    let music_asset = assets
        .get(recipe.music.asset_id.as_str())
        .filter(|asset| asset.expected_use == AssetUse::Music && asset.audio_tracks > 0)
        .ok_or("music ID must identify an asset with an audio track")?;
    if recipe.narration.asset_id == recipe.music.asset_id {
        return Err("narration and music must be separate assets");
    }

    if recipe.clips.len() != 3 {
        return Err("recipe requires exactly three ordered video shots");
    }
    let shot_order = [
        AssetUse::ShadeShot,
        AssetUse::WaterRestShot,
        AssetUse::NeighborShot,
    ];
    let mut clips = Vec::with_capacity(recipe.clips.len());
    let mut cursor = 0_u64;
    for (clip, expected_use) in recipe.clips.iter().zip(shot_order) {
        let asset = assets
            .get(clip.asset_id.as_str())
            .filter(|asset| asset.expected_use == expected_use)
            .ok_or("shot assets must match shade, water/rest, neighbor order")?;
        if clip.source_out_ms <= clip.source_in_ms
            || clip.source_out_ms > asset.duration_ms
            || clip.timeline_start_ms != cursor
        {
            return Err("clip source range is invalid or timeline cuts are not contiguous");
        }
        let length = clip.source_out_ms - clip.source_in_ms;
        let end = cursor
            .checked_add(length)
            .ok_or("timeline duration overflow")?;
        clips.push(TimelineClipPlan {
            asset_id: clip.asset_id.clone(),
            source_in_ms: clip.source_in_ms,
            source_out_ms: clip.source_out_ms,
            timeline_start_ms: cursor,
            timeline_end_ms: end,
        });
        cursor = end;
    }
    if !(20_000..=30_000).contains(&cursor) {
        return Err("compiled video duration must be between 20 and 30 seconds");
    }
    for (track, asset) in [
        (&recipe.narration, narration_asset),
        (&recipe.music, music_asset),
    ] {
        if track.source_in_ms >= track.source_out_ms
            || track.source_out_ms > asset.duration_ms
            || track.source_out_ms - track.source_in_ms != cursor
            || track.timeline_start_ms != 0
            || !(MIN_AUDIO_GAIN_MILLIDB..=MAX_AUDIO_GAIN_MILLIDB).contains(&track.gain_millidb)
        {
            return Err("audio trims must cover the timeline and use bounded gain levels");
        }
    }
    if recipe.music.gain_millidb > recipe.narration.gain_millidb - 6_000 {
        return Err("music level must remain at least 6 dB below narration");
    }

    if recipe.captions.len() != MAX_CAPTIONS
        || recipe
            .captions
            .iter()
            .map(|caption| caption.text.as_str())
            .collect::<Vec<_>>()
            != [
                "Find shade during peak heat.",
                "Drink water and take a cool break.",
                "Check on a neighbor.",
            ]
    {
        return Err("caption copy and order must exactly match the approved brief");
    }
    let mut previous_end = 0;
    for caption in &recipe.captions {
        if caption.text.len() > MAX_TEXT_BYTES
            || caption.start_ms >= caption.end_ms
            || caption.end_ms > cursor
            || caption.start_ms < previous_end
        {
            return Err(
                "caption timings must be ordered, non-overlapping, and within the timeline",
            );
        }
        previous_end = caption.end_ms;
    }
    if recipe.end_card.text != "Plan ahead. Look out for each other."
        || recipe.end_card.text.len() > MAX_TEXT_BYTES
        || recipe.end_card.start_ms < previous_end
        || recipe.end_card.end_ms != cursor
        || recipe.end_card.start_ms >= recipe.end_card.end_ms
        || recipe.end_card.end_ms - recipe.end_card.start_ms < 1_000
        || !valid_hex_color(&recipe.end_card.background_hex)
        || !valid_hex_color(&recipe.end_card.text_hex)
        || contrast_ratio(&recipe.end_card.background_hex, &recipe.end_card.text_hex)
            .is_none_or(|ratio| ratio < 4.5)
        || !(24..=120).contains(&recipe.end_card.font_size_px)
    {
        return Err(
            "visual end card must use the exact copy, fit the final interval, and have bounded styling",
        );
    }

    Ok(CapCutTimelinePlan {
        title: "Three Ways to Stay Cooler This Week".into(),
        width: recipe.width,
        height: recipe.height,
        duration_ms: cursor,
        clips,
        captions: recipe.captions.clone(),
        narration: recipe.narration.clone(),
        music: recipe.music.clone(),
        end_card: recipe.end_card.clone(),
        qualification: "offline_plan_only".into(),
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn contrast_ratio(background: &str, foreground: &str) -> Option<f64> {
    fn luminance(color: &str) -> Option<f64> {
        let bytes = color.as_bytes();
        if bytes.len() != 7 || bytes[0] != b'#' {
            return None;
        }
        let mut channels = [0.0; 3];
        for (index, channel) in channels.iter_mut().enumerate() {
            let start = 1 + index * 2;
            let value = u8::from_str_radix(std::str::from_utf8(&bytes[start..start + 2]).ok()?, 16)
                .ok()? as f64
                / 255.0;
            *channel = if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            };
        }
        Some(0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2])
    }

    let background = luminance(background)?;
    let foreground = luminance(foreground)?;
    let (lighter, darker) = if background >= foreground {
        (background, foreground)
    } else {
        (foreground, background)
    };
    Some((lighter + 0.05) / (darker + 0.05))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_recipe() -> CapCutRecipe {
        let uses = [
            AssetUse::ShadeShot,
            AssetUse::WaterRestShot,
            AssetUse::NeighborShot,
            AssetUse::Narration,
            AssetUse::Music,
        ];
        let assets = uses
            .into_iter()
            .enumerate()
            .map(|(index, expected_use)| LicensedAsset {
                asset_id: format!("asset-{index}"),
                source_and_rights: "owned test media; permission documented".into(),
                sha256: format!("{:064x}", index + 1),
                duration_ms: 30_000,
                frame_rate_milli: 30_000,
                width: 1920,
                height: 1080,
                audio_tracks: u8::from(matches!(
                    expected_use,
                    AssetUse::Narration | AssetUse::Music
                )),
                expected_use,
            })
            .collect();
        CapCutRecipe {
            assets,
            clips: vec![
                VideoClip {
                    asset_id: "asset-0".into(),
                    source_in_ms: 0,
                    source_out_ms: 7_000,
                    timeline_start_ms: 0,
                },
                VideoClip {
                    asset_id: "asset-1".into(),
                    source_in_ms: 1_000,
                    source_out_ms: 9_000,
                    timeline_start_ms: 7_000,
                },
                VideoClip {
                    asset_id: "asset-2".into(),
                    source_in_ms: 0,
                    source_out_ms: 8_000,
                    timeline_start_ms: 15_000,
                },
            ],
            captions: vec![
                Caption {
                    text: "Find shade during peak heat.".into(),
                    start_ms: 500,
                    end_ms: 6_500,
                },
                Caption {
                    text: "Drink water and take a cool break.".into(),
                    start_ms: 7_200,
                    end_ms: 14_500,
                },
                Caption {
                    text: "Check on a neighbor.".into(),
                    start_ms: 15_200,
                    end_ms: 21_500,
                },
            ],
            narration: AudioTrack {
                asset_id: "asset-3".into(),
                source_in_ms: 0,
                source_out_ms: 23_000,
                timeline_start_ms: 0,
                gain_millidb: 0,
            },
            music: AudioTrack {
                asset_id: "asset-4".into(),
                source_in_ms: 0,
                source_out_ms: 23_000,
                timeline_start_ms: 0,
                gain_millidb: -18_000,
            },
            end_card: EndCard {
                text: "Plan ahead. Look out for each other.".into(),
                start_ms: 22_000,
                end_ms: 23_000,
                background_hex: "#142B3A".into(),
                text_hex: "#FFFFFF".into(),
                font_size_px: 48,
            },
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn compiles_caller_timing_to_offline_only_23_second_plan() {
        let plan = compile_capcut_recipe(&valid_recipe()).unwrap();
        assert_eq!(plan.duration_ms, 23_000);
        assert_eq!(plan.title, "Three Ways to Stay Cooler This Week");
        assert_eq!(plan.clips[2].timeline_end_ms, 23_000);
        assert_eq!(plan.end_card.text, "Plan ahead. Look out for each other.");
        assert_eq!(plan.music.gain_millidb, -18_000);
        assert_eq!(plan.narration.source_out_ms, 23_000);
        assert_eq!(plan.qualification, "offline_plan_only");
    }

    #[test]
    fn rejects_unlicensed_or_bad_checksum_assets_and_wrong_caption_copy() {
        let mut recipe = valid_recipe();
        recipe.assets[0].source_and_rights.clear();
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.assets[0].sha256 = "not-a-checksum".into();
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.captions[0].text.push('!');
        assert!(compile_capcut_recipe(&recipe).is_err());
    }

    #[test]
    fn rejects_wrong_clip_order_gaps_and_out_of_range_captions() {
        let mut recipe = valid_recipe();
        recipe.clips.swap(0, 1);
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.clips[1].timeline_start_ms += 1;
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.end_card.end_ms = 23_001;
        assert!(compile_capcut_recipe(&recipe).is_err());
    }

    #[test]
    fn rejects_audio_without_full_trim_or_quiet_bed_and_unstyled_end_card() {
        let mut recipe = valid_recipe();
        recipe.music.source_out_ms = 22_999;
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.music.gain_millidb = -5_000;
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.end_card.background_hex = "navy".into();
        assert!(compile_capcut_recipe(&recipe).is_err());
    }

    #[test]
    fn rejects_too_short_or_low_contrast_end_card() {
        let mut recipe = valid_recipe();
        recipe.end_card.start_ms = 22_999;
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.end_card.start_ms = 23_001;
        assert!(compile_capcut_recipe(&recipe).is_err());
        let mut recipe = valid_recipe();
        recipe.end_card.text_hex = "#142B3A".into();
        assert!(compile_capcut_recipe(&recipe).is_err());
    }
}
