//! Which scripts `recognize()` actually recognises, and the limitation it has
//! for unspaced writing systems.
//!
//! The product claim is that recognition is "strong at near-duplicate/verbatim"
//! — the guardrail in `docs/pitch.md` qualifies it only as "not a paraphrase
//! matcher". These tests measure the claim directly by enrolling content and
//! recognising the BYTE-IDENTICAL string back, which is the strongest possible
//! case.
//!
//! It holds for every script tested except short Japanese and Chinese, and the
//! cause is isolated below: feature extraction is whitespace-dependent, so a
//! writing system without inter-word spaces yields fewer features per
//! character and a short passage falls under `recognize_min_score` (3.0),
//! landing on `Familiar` instead.

use spectral_recognition::{RecognitionConfig, RecognitionEngine, SqliteRecognitionStore, Verdict};
use tempfile::TempDir;

/// Enrol `content`, then recognise the byte-identical string back.
fn verbatim_verdict(content: &str) -> Verdict {
    let dir = TempDir::new().unwrap();
    let store = SqliteRecognitionStore::open(&dir.path().join("r.db")).unwrap();
    let mut engine = RecognitionEngine::new(store, RecognitionConfig::default());
    engine.enroll("m1", content).unwrap();
    engine.recognize(content).unwrap().verdict
}

fn is_recognized(v: &Verdict) -> bool {
    matches!(v, Verdict::Recognized { .. })
}

/// Scripts that separate words with spaces all satisfy the verbatim claim,
/// including non-Latin ones — so this is not a "Latin only" limitation.
#[test]
fn verbatim_recognition_holds_for_space_separated_scripts() {
    let cases: &[(&str, &str)] = &[
        (
            "english",
            "the deploy runbook lives in notion and covers rollback procedures for staging",
        ),
        (
            "russian",
            "руководство по развертыванию находится в notion и охватывает процедуры отката",
        ),
        (
            "arabic",
            "دليل النشر موجود في نوشن ويغطي إجراءات التراجع عن النشر للمرحلة التجريبية",
        ),
        (
            "korean",
            "배포 안내서는 노션에 있으며 스테이징 롤백 절차를 다룹니다 그리고 추가 설명이 있습니다",
        ),
        (
            "thai",
            "คู่มือการปรับใช้อยู่ในโนชันและครอบคลุมขั้นตอนการย้อนกลับสำหรับสเตจจิ้ง",
        ),
    ];
    for (name, text) in cases {
        assert!(
            is_recognized(&verbatim_verdict(text)),
            "{name}: byte-identical content was not Recognized"
        );
    }
}

/// **KNOWN LIMITATION.** A short Japanese or Chinese passage — roughly a
/// sentence, the common case for a single memory — is only `Familiar` when
/// recognised verbatim, never `Recognized`.
///
/// This matters because `recognize()` is the dedup / "have I seen this?"
/// primitive: a consumer keying dedup off `Recognized` will fail to dedupe
/// short CJK content and will store duplicates.
///
/// Pinned rather than fixed: the fix is a script-aware tokenizer (or a
/// character-n-gram fallback for unspaced scripts), which changes what every
/// stored fingerprint looks like and so is a re-index, not a patch.
#[test]
fn known_limitation_short_unspaced_cjk_is_familiar_not_recognized() {
    let japanese = "デプロイ手順書はNotionにあり、ステージングのロールバック手順を網羅しています";
    let chinese = "部署手册在Notion中，涵盖了暂存环境的回滚流程和相关的操作说明";

    for (name, text) in [("japanese", japanese), ("chinese", chinese)] {
        let verdict = verbatim_verdict(text);
        assert!(
            !is_recognized(&verdict),
            "{name} ({} chars) is now Recognized verbatim — the limitation this \
             test documents has been fixed; update it and the pitch guardrail",
            text.chars().count()
        );
        assert!(
            matches!(verdict, Verdict::Familiar),
            "{name}: expected Familiar (some signal, below the recognition \
             threshold), got {verdict:?}"
        );
    }
}

/// The cause is **whitespace**, not the script and not length alone.
///
/// English at 34 characters is Recognized while Japanese at 41 is not, so it
/// is not a pure length threshold. Inserting spaces into the *same* Japanese
/// text makes it Recognized, which isolates tokenization as the mechanism:
/// feature extraction is whitespace-dependent, so an unspaced script yields
/// fewer features per character and falls under `recognize_min_score`.
#[test]
fn the_cause_is_whitespace_tokenization_not_the_script_itself() {
    let english_short = "the deploy runbook lives in notion"; // 34 chars
    assert!(
        is_recognized(&verbatim_verdict(english_short)),
        "precondition: short English should be Recognized"
    );

    let japanese = "デプロイ手順書はNotionにあり、ステージングのロールバック手順を網羅しています";
    assert!(
        japanese.chars().count() > english_short.chars().count(),
        "precondition: the Japanese sample is the LONGER of the two"
    );
    assert!(
        !is_recognized(&verbatim_verdict(japanese)),
        "precondition: the unspaced Japanese sample is not Recognized"
    );

    // Same words, spaces inserted.
    let japanese_spaced =
        "デプロイ 手順書 は Notion に あり ステージング の ロールバック 手順 を 網羅 し て い ます";
    assert!(
        is_recognized(&verbatim_verdict(japanese_spaced)),
        "adding whitespace to the same Japanese text did not make it \
         Recognized — the cause is not tokenization after all, and this \
         diagnosis needs revisiting"
    );
}

/// The limitation is bounded: longer CJK passages DO recognise, so this is a
/// short-content problem rather than a total failure for those scripts.
#[test]
fn longer_cjk_passages_are_recognized() {
    let japanese = "デプロイ手順書はNotionにあり、ステージングのロールバック手順を網羅しています";
    let chinese = "部署手册在Notion中，涵盖了暂存环境的回滚流程和相关的操作说明";

    for (name, text) in [("japanese", japanese), ("chinese", chinese)] {
        let doubled = text.repeat(2);
        assert!(
            is_recognized(&verbatim_verdict(&doubled)),
            "{name}: even at {} chars the passage is not Recognized",
            doubled.chars().count()
        );
    }
}
