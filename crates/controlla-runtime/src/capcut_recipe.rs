//! Offline CapCut Web recipe validation. This compiles caller-supplied media metadata and timing
//! into a bounded plan; it does not resolve controls or execute anything in CapCut.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ASSETS: usize = 5;
const MAX_CAPTIONS: usize = 4;
const MAX_ID_LEN: usize = 80;
const MAX_TEXT_BYTES: usize = 500;

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
pub struct CapCutRecipe {
    pub assets: Vec<LicensedAsset>,
    pub clips: Vec<VideoClip>,
    pub captions: Vec<Caption>,
    pub narration_asset_id: String,
    pub music_asset_id: String,
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
    pub narration_asset_id: String,
    pub music_asset_id: String,
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
    let narration = assets
        .get(recipe.narration_asset_id.as_str())
        .filter(|asset| asset.expected_use == AssetUse::Narration && asset.audio_tracks > 0)
        .ok_or("narration ID must identify an asset with an audio track")?;
    let music = assets
        .get(recipe.music_asset_id.as_str())
        .filter(|asset| asset.expected_use == AssetUse::Music && asset.audio_tracks > 0)
        .ok_or("music ID must identify an asset with an audio track")?;
    if recipe.narration_asset_id == recipe.music_asset_id {
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
    if narration.duration_ms < cursor || music.duration_ms < cursor {
        return Err("narration and music assets must cover the complete timeline");
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
                "Plan ahead. Look out for each other.",
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

    Ok(CapCutTimelinePlan {
        title: "Three Ways to Stay Cooler This Week".into(),
        width: recipe.width,
        height: recipe.height,
        duration_ms: cursor,
        clips,
        captions: recipe.captions.clone(),
        narration_asset_id: recipe.narration_asset_id.clone(),
        music_asset_id: recipe.music_asset_id.clone(),
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
                Caption {
                    text: "Plan ahead. Look out for each other.".into(),
                    start_ms: 22_000,
                    end_ms: 23_000,
                },
            ],
            narration_asset_id: "asset-3".into(),
            music_asset_id: "asset-4".into(),
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
        assert_eq!(
            plan.captions[3].text,
            "Plan ahead. Look out for each other."
        );
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
        recipe.captions[3].end_ms = 23_001;
        assert!(compile_capcut_recipe(&recipe).is_err());
    }
}
