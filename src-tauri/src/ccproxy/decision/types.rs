//! Typed System One questions, answers and failure categories.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Question {
    Choice { instructions: String, criteria: BTreeMap<String, String> },
    Noul { instructions: String },
    Score { instructions: String, criteria: Vec<String> },
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DecisionRequest {
    pub state: String,
    pub model: String,
    pub questions: BTreeMap<String, Question>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Answer {
    Choice { choice: String, probabilities: BTreeMap<String, f64>, confidence: f64 },
    Noul { noul: f64 },
    Score { score: f64, legend: BTreeMap<String, String>, probabilities: BTreeMap<String, f64>, confidence: f64 },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DecisionResponse {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DecisionError {
    #[error("invalid decision endpoint: {0}")]
    InvalidUrl(&'static str),
    #[error("decision adapter catalog error: {0}")]
    Catalog(String),
    #[error("decision provider/model is unavailable")]
    Unavailable,
    #[error("decision request is invalid: {0}")]
    InvalidRequest(&'static str),
    #[error("decision transport failed: {0}")]
    Transport(&'static str),
    #[error("decision upstream returned HTTP {0}")]
    Http(u16),
    #[error("decision upstream response is invalid: {0}")]
    InvalidResponse(&'static str),
    #[error("decision model listing cannot be derived from this endpoint")]
    UnsupportedListEndpoint,
}

impl DecisionRequest {
    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        if self.model.trim().is_empty() || self.state.len() > 65_536 || self.questions.is_empty() || self.questions.len() > 8 {
            return Err(DecisionError::InvalidRequest("model, state or question count"));
        }
        for (id, question) in &self.questions {
            if id.trim().is_empty() || id.len() > 64 {
                return Err(DecisionError::InvalidRequest("question id"));
            }
            match question {
                Question::Choice { instructions, criteria } if instructions.trim().is_empty() || criteria.len() < 2 || criteria.len() > 255 || criteria.keys().any(|key| key.trim().is_empty()) => return Err(DecisionError::InvalidRequest("choice criteria")),
                Question::Noul { instructions } if instructions.trim().is_empty() => return Err(DecisionError::InvalidRequest("noul instructions")),
                Question::Score { instructions, criteria } if instructions.trim().is_empty() || !(2..=10).contains(&criteria.len()) => return Err(DecisionError::InvalidRequest("score criteria")),
                _ => {}
            }
        }
        Ok(())
    }
}

impl DecisionResponse {
    pub(crate) fn validate(&self, request: &DecisionRequest) -> Result<(), DecisionError> {
        if self.model.trim().is_empty() || self.answers.len() != request.questions.len() {
            return Err(DecisionError::InvalidResponse("model or answers"));
        }
        for (id, question) in &request.questions {
            let answer = self.answers.get(id).ok_or(DecisionError::InvalidResponse("missing answer"))?;
            match (question, answer) {
                (Question::Choice { criteria, .. }, Answer::Choice { choice, probabilities, confidence }) => {
                    if !criteria.contains_key(choice) || probabilities.len() != criteria.len() || probabilities.keys().any(|key| !criteria.contains_key(key)) || !valid_probability(*confidence) || !valid_distribution(probabilities.values().copied()) || probabilities.get(choice).is_none_or(|selected| probabilities.values().any(|other| other > selected)) {
                        return Err(DecisionError::InvalidResponse("choice probabilities"));
                    }
                }
                (Question::Noul { .. }, Answer::Noul { noul }) if valid_probability(*noul) => {}
                (Question::Score { criteria, .. }, Answer::Score { score, legend, probabilities, confidence }) => {
                    if !score.is_finite() || *score < 0.0 || *score > (criteria.len() - 1) as f64 || !valid_probability(*confidence) || legend.len() != criteria.len() || legend.iter().any(|(key, value)| key.parse::<usize>().ok().and_then(|index| criteria.get(index)).is_none_or(|criterion| criterion != value)) || probabilities.len() != criteria.len() || probabilities.keys().any(|key| !legend.contains_key(key)) || !valid_distribution(probabilities.values().copied()) {
                        return Err(DecisionError::InvalidResponse("score probabilities"));
                    }
                }
                _ => return Err(DecisionError::InvalidResponse("answer type")),
            }
        }
        Ok(())
    }
}

fn valid_probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn valid_distribution(values: impl Iterator<Item = f64>) -> bool {
    let mut total = 0.0;
    for value in values {
        if !valid_probability(value) { return false; }
        total += value;
    }
    (total - 1.0).abs() <= 0.01
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_validation_tolerates_upstream_probability_rounding() {
        use serde_json::json;
        // Shape of a real /v1/systemone answer for a 17-option Choice: every option is reported and
        // the probabilities sum to 0.9997, so the distribution check must keep a rounding tolerance.
        let criteria: BTreeMap<String, String> = (0..16)
            .map(|index| (format!("report_{index}"), "faithful report".to_string()))
            .chain(std::iter::once((
                "ambiguous".to_string(),
                "cannot decide".to_string(),
            )))
            .collect();
        let request = DecisionRequest {
            state: "candidates".into(),
            model: "jev-latest".into(),
            questions: BTreeMap::from([(
                "report_selection".into(),
                Question::Choice {
                    instructions: "choose".into(),
                    criteria: criteria.clone(),
                },
            )]),
        };
        let mut probabilities: BTreeMap<String, f64> = BTreeMap::new();
        probabilities.insert("ambiguous".to_string(), 0.1497);
        probabilities.insert("report_0".to_string(), 0.4);
        for index in 1..16 {
            probabilities.insert(format!("report_{index}"), 0.03);
        }
        let body = json!({
            "model": "jev-1.13.0",
            "answers": {"report_selection": {
                "type": "choice",
                "choice": "report_0",
                "probabilities": probabilities,
                "confidence": 0.5,
            }},
            "usage": {"input_tokens": 1, "output_tokens": 1},
        });
        let valid: DecisionResponse = serde_json::from_value(body.clone()).unwrap();
        valid
            .validate(&request)
            .expect("upstream rounding must stay inside the tolerance");

        let mut out_of_tolerance = body.clone();
        out_of_tolerance["answers"]["report_selection"]["probabilities"]["ambiguous"] = json!(0.13);
        assert!(
            serde_json::from_value::<DecisionResponse>(out_of_tolerance)
                .unwrap()
                .validate(&request)
                .is_err(),
            "a distribution outside the tolerance must be rejected"
        );
    }

    #[test]
    fn typed_answers_reject_mismatch_and_bad_probabilities() {
        use serde_json::json;
        let questions = BTreeMap::from([
            ("language".into(), Question::Choice { instructions: "language".into(), criteria: BTreeMap::from([("en".into(), "English".into()), ("zh".into(), "Chinese".into())]) }),
            ("urgent".into(), Question::Noul { instructions: "urgent".into() }),
            ("risk".into(), Question::Score { instructions: "risk".into(), criteria: vec!["low".into(), "high".into()] }),
        ]);
        let request = DecisionRequest { state: "hello".into(), model: "jev-latest".into(), questions };
        request.validate().unwrap();
        let body = json!({"model":"jev-1.13.0","answers":{"language":{"type":"choice","choice":"en","confidence":0.9,"probabilities":{"en":0.9,"zh":0.1}},"urgent":{"type":"noul","noul":0.8},"risk":{"type":"score","score":0.8,"legend":{"0":"low","1":"high"},"confidence":0.8,"probabilities":{"0":0.2,"1":0.8}}},"usage":{"input_tokens":4,"output_tokens":3}});
        let valid: DecisionResponse = serde_json::from_value(body.clone()).unwrap();
        valid.validate(&request).unwrap();
        let mut bad_legend = body.clone();
        bad_legend["answers"]["risk"]["legend"]["1"] = json!("unexpected");
        assert!(serde_json::from_value::<DecisionResponse>(bad_legend).unwrap().validate(&request).is_err());
        let mut bad = body.clone();
        bad["answers"]["language"]["type"] = json!("noul");
        assert!(serde_json::from_value::<DecisionResponse>(bad).is_err());
        let mut bad = body.clone();
        bad["answers"]["language"]["probabilities"]["en"] = json!(1.5);
        assert!(serde_json::from_value::<DecisionResponse>(bad).unwrap().validate(&request).is_err());
        let mut bad = body;
        bad["usage"] = json!({});
        assert!(serde_json::from_value::<DecisionResponse>(bad).is_err());
    }
}
