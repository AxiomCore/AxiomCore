//! Optional remote natural-language planning for Axiom Inspector.
//!
//! Jev is allowed to choose only a supported intent and a member of a
//! deterministic candidate set. The returned request still passes the normal
//! `QuestionRequest` validation and all facts come from the local EvidenceIndex.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use axiom_lib::{
    application_evidence::{ApplicationEvidence, EvidenceNode},
    question::{QuestionIntent, QuestionRequest, QUESTION_FORMAT},
};
use chrono::Utc;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const JEV_MODEL: &str = "typesafe-ai/jev";
pub const RECEIPT_FORMAT: &str = "axiom-jev-planner-receipt/v1";
const GATEWAY_ENDPOINT: &str = "https://ai-gateway.vercel.sh/v1/evaluate";
const MAX_CANDIDATES: usize = 48;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JevCandidate {
    pub key: String,
    pub id: String,
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JevThresholds {
    pub intent: f64,
    pub selector: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JevDecision {
    pub intent: String,
    pub intent_probability: f64,
    pub selector: Option<String>,
    pub selector_probability: Option<f64>,
    pub selector_source: String,
    pub evaluated_selector: Option<String>,
    pub evaluated_selector_probability: Option<f64>,
    pub accepted: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JevPlannerReceipt {
    pub format: String,
    pub provider: String,
    pub requested_model: String,
    pub resolved_model: String,
    pub graph_revision: String,
    pub question_sha256: String,
    pub candidate_count: usize,
    pub candidates: Vec<JevCandidate>,
    pub thresholds: JevThresholds,
    pub decision: JevDecision,
    pub request: Option<QuestionRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GatewayResponse {
    model: String,
    answers: BTreeMap<String, ChoiceAnswer>,
    usage: Option<Value>,
    provider_metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ChoiceAnswer {
    #[serde(rename = "type")]
    answer_type: String,
    choice: String,
    probabilities: BTreeMap<String, f64>,
}

pub async fn plan(
    evidence: &ApplicationEvidence,
    question: &str,
    minimum_intent_probability: f64,
    minimum_selector_probability: f64,
) -> Result<JevPlannerReceipt> {
    validate_threshold("intent", minimum_intent_probability)?;
    validate_threshold("selector", minimum_selector_probability)?;
    if question.trim().is_empty() || question.len() > 512 || question.chars().any(char::is_control)
    {
        bail!("question must be one printable line of at most 512 bytes");
    }
    let api_key = env::var("AI_GATEWAY_API_KEY").context(
        "AI_GATEWAY_API_KEY is not injected; run this command through Infisical or export it locally",
    )?;
    if api_key.trim().is_empty() {
        bail!("AI_GATEWAY_API_KEY is empty");
    }

    let candidates = candidate_shortlist(evidence, question);
    if candidates.is_empty() {
        bail!("the evidence graph has no semantic candidates");
    }
    let selector_criteria = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.key.clone(),
                json!({
                    "targetKind": candidate.kind,
                    "targetName": candidate.label,
                    "semanticId": candidate.id,
                    "chooseWhen": "This is the subject explicitly named by the user's question.",
                    "doNotChooseWhen": "This merely writes, calls, authorizes, renders, or otherwise answers a question about a different target."
                }),
            )
        })
        .chain(std::iter::once((
            "none".into(),
            json!({
                "chooseWhen": "No supplied candidate is explicitly and unambiguously the subject of the user's question."
            }),
        )))
        .collect::<BTreeMap<String, Value>>();
    let payload = json!({
        "model": JEV_MODEL,
        "state": {
            "userQuestion": question,
            "axiomInspectorContext": {
                "purpose": "Axiom Inspector answers questions from a deterministic semantic application graph.",
                "targetMeaning": "The target is the application entity the user asks about, not an entity that may be part of the answer.",
                "kinds": {
                    "state": "A declared frontend or backend state value.",
                    "extension": "A capability-scoped imperative WASM module.",
                    "operation": "A typed frontend or backend contract operation.",
                    "action": "A declared application action.",
                    "permission": "Authority evidence that may explain access; it is not the target unless the user explicitly names the permission."
                }
            },
            "supportedIntents": ["writers", "network-access", "execution", "why", "changes", "unsupported"],
            "candidateTargets": candidates,
            "safetyBoundary": "Select only from the supplied intent and candidate names. Do not infer new application facts."
        },
        "questions": {
            "intent": {
                "type": "choice",
                "instructions": "Classify the user question into exactly one supported Axiom Inspector intent. Use unsupported when the question cannot be represented by the listed intents.",
                "criteria": {
                    "writers": "The user asks which declared code, action, extension, operation, or authority can write, mutate, update, or change one application fact.",
                    "network-access": "The user asks why or whether one extension or component can access the network.",
                    "execution": "The user asks what runs, executes, is invoked, or is triggered by an application action or event.",
                    "why": "The user asks why one semantic fact exists, is connected, is allowed, or affects another fact.",
                    "changes": "The user asks what changed semantically between the current application and an available baseline.",
                    "unsupported": "The request is outside the five supported intents, is conversational, asks for generation, or cannot be represented safely."
                }
            },
            "selector": {
                "type": "choice",
                "instructions": {
                    "question": "Which entry in `candidateTargets` is the target explicitly named by `userQuestion`?",
                    "rules": [
                        "Choose the subject being investigated, not a node that could answer the question.",
                        "For 'who can modify X', X is the target; a writer or write permission is an answer, not the target.",
                        "For 'can X access the network', X is the target.",
                        "Use none if a unique target is not explicit."
                    ]
                },
                "criteria": selector_criteria
            }
        },
        "providerOptions": {
            "gateway": { "only": ["typesafe-ai"] }
        }
    });

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?
        .post(GATEWAY_ENDPOINT)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .context("contact Vercel AI Gateway Jev evaluation endpoint")?;
    let status = response.status();
    if status != StatusCode::OK {
        // Do not include a provider response body: it can contain request state
        // and is not needed to diagnose the status class.
        match status {
            StatusCode::UNAUTHORIZED => bail!(
                "Vercel AI Gateway rejected AI_GATEWAY_API_KEY; inject an AI Gateway API key, not a general Vercel or provider token"
            ),
            StatusCode::FORBIDDEN => bail!(
                "Vercel AI Gateway denied evaluation access (HTTP 403); confirm the key's team has AI Gateway credits and access to typesafe-ai/jev"
            ),
            StatusCode::TOO_MANY_REQUESTS => {
                bail!("Vercel AI Gateway rate-limited the Jev evaluation (HTTP 429)")
            }
            _ => bail!("Vercel AI Gateway Jev request failed with HTTP {status}"),
        }
    }
    let result: GatewayResponse = response
        .json()
        .await
        .context("decode Vercel AI Gateway Jev response")?;
    decide(
        evidence,
        question,
        candidates,
        minimum_intent_probability,
        minimum_selector_probability,
        result,
    )
}

fn decide(
    evidence: &ApplicationEvidence,
    question: &str,
    candidates: Vec<JevCandidate>,
    intent_threshold: f64,
    selector_threshold: f64,
    response: GatewayResponse,
) -> Result<JevPlannerReceipt> {
    let intent_answer = response
        .answers
        .get("intent")
        .context("Jev response omitted intent answer")?;
    let selector_answer = response
        .answers
        .get("selector")
        .context("Jev response omitted selector answer")?;
    if intent_answer.answer_type != "choice" || selector_answer.answer_type != "choice" {
        bail!("Jev returned a non-choice planner answer");
    }
    let intent = intent_answer.choice.as_str();
    let intent_probability = probability(intent_answer);
    let selector_probability = probability(selector_answer);
    let evaluated_candidate = candidates
        .iter()
        .find(|candidate| candidate.key == selector_answer.choice);
    let evaluated_candidate_id = evaluated_candidate.map(|candidate| candidate.id.clone());
    // Exact identifiers are software syntax, so resolve them in code instead of
    // asking a probabilistic model to rediscover a deterministic equality. Jev
    // remains responsible for the natural-language intent. Fuzzy or ambiguous
    // targets still require its bounded selector and confidence gate.
    let exact_candidate = deterministic_exact_selector(&candidates, question);
    let (candidate, selector_source, selected_probability) = if intent == "changes" {
        (None, "not-applicable", None)
    } else {
        match exact_candidate {
            Some(candidate)
                if evaluated_candidate.map(|value| &value.id) == Some(&candidate.id) =>
            {
                (
                    Some(candidate),
                    "deterministic-exact+jev",
                    Some(selector_probability),
                )
            }
            Some(candidate) => (Some(candidate), "deterministic-exact", None),
            None => (evaluated_candidate, "jev", Some(selector_probability)),
        }
    };
    let candidate_id = candidate.map(|candidate| candidate.id.clone());
    let supported = matches!(
        intent,
        "writers" | "network-access" | "execution" | "why" | "changes"
    );
    let (accepted, reason) = if !supported || intent == "unsupported" {
        (false, "Jev classified the question as unsupported.")
    } else if intent_probability < intent_threshold {
        (
            false,
            "Intent probability is below the configured threshold.",
        )
    } else if intent != "changes" && candidate.is_none() {
        (
            false,
            "Jev did not select an allowlisted semantic candidate.",
        )
    } else if intent != "changes"
        && selector_source == "jev"
        && selector_probability < selector_threshold
    {
        (
            false,
            "Selector probability is below the configured threshold.",
        )
    } else if selector_source.starts_with("deterministic-exact") {
        (
            true,
            "Jev selected a supported intent above the configured threshold; Axiom resolved the explicitly named semantic target exactly.",
        )
    } else {
        (
            true,
            "Jev selected a supported intent and an allowlisted semantic candidate above the configured thresholds.",
        )
    };
    let request = if accepted {
        let intent = match intent {
            "writers" => QuestionIntent::Writers {
                selector: candidate.context("missing writers selector")?.id.clone(),
            },
            "network-access" => QuestionIntent::NetworkAccess {
                selector: candidate.context("missing network selector")?.id.clone(),
            },
            "execution" => QuestionIntent::Execution {
                selector: candidate.context("missing execution selector")?.id.clone(),
            },
            "why" => QuestionIntent::Why {
                selector: candidate.context("missing why selector")?.id.clone(),
            },
            "changes" => QuestionIntent::Changes,
            _ => unreachable!(),
        };
        let request = QuestionRequest {
            format: QUESTION_FORMAT.into(),
            intent,
            max_depth: 8,
            max_results: 250,
        };
        request.validate()?;
        Some(request)
    } else {
        None
    };
    Ok(JevPlannerReceipt {
        format: RECEIPT_FORMAT.into(),
        provider: "vercel-ai-gateway".into(),
        requested_model: JEV_MODEL.into(),
        resolved_model: response.model,
        graph_revision: evidence.graph_revision.clone(),
        question_sha256: hex::encode(Sha256::digest(question.as_bytes())),
        candidate_count: candidates.len(),
        candidates,
        thresholds: JevThresholds {
            intent: intent_threshold,
            selector: selector_threshold,
        },
        decision: JevDecision {
            intent: intent.into(),
            intent_probability,
            selector: candidate_id,
            selector_probability: (intent != "changes")
                .then_some(selected_probability)
                .flatten(),
            selector_source: selector_source.into(),
            evaluated_selector: evaluated_candidate_id,
            evaluated_selector_probability: (intent != "changes").then_some(selector_probability),
            accepted,
            reason: reason.into(),
        },
        request,
        usage: response.usage,
        provider_metadata: response.provider_metadata,
    })
}

pub fn write_receipt(workspace: &Path, receipt: &JevPlannerReceipt) -> Result<PathBuf> {
    let directory = workspace.join(".axiom/inspector/planner-receipts");
    fs::create_dir_all(&directory)?;
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let path = directory.join(format!(
        "jev-{timestamp}-{}.json",
        &receipt.question_sha256[..12]
    ));
    fs::write(&path, serde_jcs::to_vec(receipt)?)
        .with_context(|| format!("write Jev planner receipt {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

fn validate_threshold(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        bail!("Jev {name} threshold must be between 0 and 1");
    }
    Ok(())
}

fn probability(answer: &ChoiceAnswer) -> f64 {
    answer
        .probabilities
        .get(&answer.choice)
        .copied()
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .unwrap_or(0.0)
}

fn candidate_shortlist(evidence: &ApplicationEvidence, question: &str) -> Vec<JevCandidate> {
    let terms = tokens(question);
    let normalized_question = normalize(question);
    let mut ranked = evidence
        .nodes
        .iter()
        .map(|node| (candidate_score(node, &normalized_question, &terms), node))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(score, node)| (Reverse(*score), node.label.clone(), node.id.clone()));
    let relevant = ranked.iter().filter(|(score, _)| *score > 0).count();
    let take = relevant.max(12).min(MAX_CANDIDATES).min(ranked.len());
    ranked
        .into_iter()
        .take(take)
        .enumerate()
        .map(|(index, (_, node))| JevCandidate {
            key: format!("c{index}"),
            id: node.id.clone(),
            kind: kind_name(node),
            label: node.label.clone(),
        })
        .collect()
}

fn deterministic_exact_selector<'a>(
    candidates: &'a [JevCandidate],
    question: &str,
) -> Option<&'a JevCandidate> {
    let question = format!(" {} ", normalize(question));
    let mut matches = candidates
        .iter()
        .filter_map(|candidate| {
            let label = normalize(&candidate.label);
            (!label.is_empty() && question.contains(&format!(" {label} ")))
                .then_some((label.len(), candidate))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(length, candidate)| (Reverse(*length), candidate.id.clone()));
    let (best_length, best) = matches.first().copied()?;
    let equally_specific = matches
        .iter()
        .filter(|(length, _)| *length == best_length)
        .count();
    (equally_specific == 1).then_some(best)
}

fn candidate_score(node: &EvidenceNode, question: &str, terms: &BTreeSet<String>) -> usize {
    let label = normalize(&node.label);
    let id = normalize(&node.id);
    let kind = kind_name(node);
    let mut score = 0;
    if !label.is_empty() && question.contains(&label) {
        score += 100;
    }
    for term in terms.iter().filter(|term| term.len() >= 3) {
        if label.contains(term) {
            score += 12;
        } else if id.contains(term) {
            score += 6;
        }
        if kind.contains(term) {
            score += 2;
        }
    }
    score
}

fn kind_name(node: &EvidenceNode) -> String {
    serde_json::to_value(node.kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "fact".into())
}

fn tokens(value: &str) -> BTreeSet<String> {
    normalize(value)
        .split_whitespace()
        .filter(|token| !STOP_WORDS.contains(token))
        .map(str::to_owned)
        .collect()
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '_' {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "can", "could", "does", "for", "have", "i", "in", "is", "it", "of", "on",
    "or", "the", "this", "to", "what", "when", "which", "who", "why", "with",
];

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_lib::application_evidence::{
        semantic_id, EvidenceNodeKind, TruthLayer, VerificationState,
    };

    fn evidence() -> ApplicationEvidence {
        let mut evidence = ApplicationEvidence::new(".", "test");
        evidence.nodes = vec![
            EvidenceNode {
                id: semantic_id("state", "cart.total"),
                kind: EvidenceNodeKind::State,
                label: "cart.total".into(),
                layer: TruthLayer::Declared,
                verification: VerificationState::Verified,
                targets: vec![],
                attributes: BTreeMap::new(),
                evidence: vec![],
            },
            EvidenceNode {
                id: semantic_id("operation", "checkout"),
                kind: EvidenceNodeKind::Operation,
                label: "Checkout".into(),
                layer: TruthLayer::Declared,
                verification: VerificationState::Verified,
                targets: vec![],
                attributes: BTreeMap::new(),
                evidence: vec![],
            },
        ];
        evidence.finalize().unwrap()
    }

    #[test]
    fn shortlist_prefers_question_terms_and_is_allowlisted() {
        let candidates = candidate_shortlist(&evidence(), "what can write cart total");
        assert_eq!(candidates[0].id, semantic_id("state", "cart.total"));
        assert_eq!(candidates[0].key, "c0");
        assert!(candidates.len() <= MAX_CANDIDATES);
    }

    #[test]
    fn exact_selector_uses_the_uniquely_named_semantic_target() {
        let candidates = candidate_shortlist(&evidence(), "who can modify cart.total");
        let selected = deterministic_exact_selector(&candidates, "who can modify cart.total")
            .expect("exact selector");
        assert_eq!(selected.id, semantic_id("state", "cart.total"));
    }

    #[test]
    fn exact_selector_abstains_when_the_same_label_is_ambiguous() {
        let mut candidates = candidate_shortlist(&evidence(), "what is checkout");
        let mut duplicate = candidates
            .iter()
            .find(|candidate| candidate.label == "Checkout")
            .unwrap()
            .clone();
        duplicate.id = "operation:duplicate".into();
        candidates.push(duplicate);
        assert!(deterministic_exact_selector(&candidates, "what is checkout").is_none());
    }

    #[test]
    fn decision_accepts_only_supported_allowlisted_results() {
        let evidence = evidence();
        let candidates = candidate_shortlist(&evidence, "what can write cart total");
        let response = GatewayResponse {
            model: JEV_MODEL.into(),
            answers: BTreeMap::from([
                (
                    "intent".into(),
                    ChoiceAnswer {
                        answer_type: "choice".into(),
                        choice: "writers".into(),
                        probabilities: BTreeMap::from([("writers".into(), 0.95)]),
                    },
                ),
                (
                    "selector".into(),
                    ChoiceAnswer {
                        answer_type: "choice".into(),
                        choice: "c0".into(),
                        probabilities: BTreeMap::from([("c0".into(), 0.9)]),
                    },
                ),
            ]),
            usage: None,
            provider_metadata: None,
        };
        let receipt = decide(
            &evidence,
            "what can write cart total",
            candidates,
            0.65,
            0.55,
            response,
        )
        .unwrap();
        assert!(receipt.decision.accepted);
        assert!(matches!(
            receipt.request.unwrap().intent,
            QuestionIntent::Writers { .. }
        ));
        assert_eq!(receipt.question_sha256.len(), 64);
    }

    #[test]
    fn decision_abstains_below_threshold() {
        let evidence = evidence();
        let candidates = candidate_shortlist(&evidence, "what can write cart total");
        let response = GatewayResponse {
            model: JEV_MODEL.into(),
            answers: BTreeMap::from([
                (
                    "intent".into(),
                    ChoiceAnswer {
                        answer_type: "choice".into(),
                        choice: "writers".into(),
                        probabilities: BTreeMap::from([("writers".into(), 0.5)]),
                    },
                ),
                (
                    "selector".into(),
                    ChoiceAnswer {
                        answer_type: "choice".into(),
                        choice: "c0".into(),
                        probabilities: BTreeMap::from([("c0".into(), 0.9)]),
                    },
                ),
            ]),
            usage: None,
            provider_metadata: None,
        };
        let receipt = decide(
            &evidence,
            "what can write cart total",
            candidates,
            0.65,
            0.55,
            response,
        )
        .unwrap();
        assert!(!receipt.decision.accepted);
        assert!(receipt.request.is_none());
    }
}
