//! Speech to text through a provider's audio transcription endpoint: the
//! person's own OpenAI or Groq key. The audio is sent once and never
//! retried; nothing of it is kept or logged.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::Url;
use serde_json::Value;
use zephium_core::work::model::{WorkModelError, WorkModelProvider};

use super::{bounded_body, http, is_loopback, join, LeadCredential, GROQ_BASE, OPENAI_BASE};

/// OpenAI's and Groq's upload ceiling.
const PROVIDER_MAX_BYTES: usize = 25 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(90);
const OPENAI_MODEL: &str = "gpt-4o-transcribe";
const GROQ_MODEL: &str = "whisper-large-v3-turbo";

/// Where one transcription goes.
#[derive(Clone, Debug)]
pub struct TranscriptionTarget {
    url: Url,
    model: String,
    max_bytes: usize,
}

impl TranscriptionTarget {
    /// The provider's own endpoint with its default speech model.
    pub fn direct(provider: WorkModelProvider) -> Result<Self, WorkModelError> {
        let (base, model) = match provider {
            WorkModelProvider::OpenAi => (OPENAI_BASE, OPENAI_MODEL),
            WorkModelProvider::Groq => (GROQ_BASE, GROQ_MODEL),
            _ => return Err(WorkModelError::BadRequest),
        };
        let base = Url::parse(base).map_err(|_| WorkModelError::BadRequest)?;
        Ok(Self {
            url: join(&base, "audio/transcriptions")?,
            model: model.to_owned(),
            max_bytes: PROVIDER_MAX_BYTES,
        })
    }

    /// The endpoint this target posts to.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The model named in the upload.
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// What came back.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Transcript {
    /// The words, trimmed.
    pub text: String,
}

fn part(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    );
}

fn multipart(boundary: &str, model: &str, language: Option<&str>, wav: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 640);
    part(&mut body, boundary, "model", model);
    part(&mut body, boundary, "response_format", "json");
    if let Some(language) = language {
        part(&mut body, boundary, "language", language);
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// An ISO 639-1 code, the only language hint the endpoints take.
fn language_hint(language: Option<&str>) -> Option<String> {
    let code = language?.split(['-', '_']).next()?.to_ascii_lowercase();
    (code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase())).then_some(code)
}

/// Sends one WAV take. `language` (a BCP 47 tag) is a hint; without it the
/// model detects the language.
pub async fn transcribe(
    target: &TranscriptionTarget,
    credential: &dyn LeadCredential,
    wav: &[u8],
    language: Option<&str>,
) -> Result<Transcript, WorkModelError> {
    if wav.is_empty() {
        return Err(WorkModelError::BadRequest);
    }
    let boundary = format!("zephium-{}", ulid::Ulid::new());
    let body = multipart(
        &boundary,
        &target.model,
        language_hint(language).as_deref(),
        wav,
    );
    if body.len() > target.max_bytes {
        return Err(WorkModelError::ContextTooLong);
    }
    let secret = credential.secret().await?;
    let mut headers = HeaderMap::new();
    headers.insert("authorization", secret.header("Bearer ")?);
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&format!("multipart/form-data; boundary={boundary}"))
            .map_err(|_| WorkModelError::Protocol)?,
    );
    let response = http(is_loopback(&target.url))?
        .post(target.url.clone())
        .headers(headers)
        .timeout(TIMEOUT)
        .body(body)
        .send()
        .await
        .map_err(|error| {
            if error.is_builder() {
                WorkModelError::Protocol
            } else {
                WorkModelError::Network
            }
        })?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = bounded_body(response).await;
    if !status.is_success() {
        let (error, _) = super::refused(status, &headers, &body);
        if error == WorkModelError::Unauthorized {
            credential.rejected();
        }
        return Err(error);
    }
    let json: Value = serde_json::from_slice(&body).map_err(|_| WorkModelError::Protocol)?;
    let text = json
        .get("text")
        .and_then(Value::as_str)
        .ok_or(WorkModelError::Protocol)?
        .trim();
    if text.len() > MAX_TEXT_BYTES {
        return Err(WorkModelError::Protocol);
    }
    Ok(Transcript {
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{http_reply, serve};
    use super::super::{LeadSecret, LeadStaticCredential};
    use super::*;

    fn credential() -> LeadStaticCredential {
        LeadStaticCredential::new(LeadSecret::new("sk-test".into()).unwrap())
    }

    fn at(base: &str) -> TranscriptionTarget {
        TranscriptionTarget {
            url: Url::parse(&format!("{base}/audio/transcriptions")).unwrap(),
            model: "gpt-4o-transcribe".into(),
            max_bytes: PROVIDER_MAX_BYTES,
        }
    }

    #[test]
    fn targets_name_each_providers_endpoint_and_model() {
        let openai = TranscriptionTarget::direct(WorkModelProvider::OpenAi).unwrap();
        assert_eq!(
            openai.url().as_str(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
        assert_eq!(openai.model(), OPENAI_MODEL);
        let groq = TranscriptionTarget::direct(WorkModelProvider::Groq).unwrap();
        assert_eq!(
            groq.url().as_str(),
            "https://api.groq.com/openai/v1/audio/transcriptions"
        );
        assert_eq!(groq.model(), GROQ_MODEL);
        assert!(TranscriptionTarget::direct(WorkModelProvider::Anthropic).is_err());
    }

    #[test]
    fn the_language_hint_is_a_bare_iso_639_1_code() {
        assert_eq!(language_hint(Some("pl-PL")).as_deref(), Some("pl"));
        assert_eq!(language_hint(Some("en_US")).as_deref(), Some("en"));
        assert_eq!(language_hint(Some("yue")), None);
        assert_eq!(language_hint(None), None);
    }

    #[tokio::test]
    async fn a_direct_take_is_one_multipart_upload_with_the_key() {
        let (base, seen) = serve(vec![http_reply(
            "200 OK",
            "content-type: application/json\r\n",
            "{\"text\":\" Book a table for two. \"}",
        )]);
        let target = at(&base);
        let wav = zephium_wav();
        let transcript = transcribe(&target, &credential(), &wav, Some("en-US"))
            .await
            .unwrap();
        assert_eq!(transcript.text, "Book a table for two.");
        let request = seen.lock().unwrap()[0].clone();
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /v1/audio/transcriptions "));
        assert!(lower.contains("authorization: bearer sk-test"));
        assert!(lower.contains("content-type: multipart/form-data; boundary=zephium-"));
        assert!(!lower.contains("idempotency-key"));
        assert!(request.contains("name=\"model\"\r\n\r\ngpt-4o-transcribe\r\n"));
        assert!(request.contains("name=\"language\"\r\n\r\nen\r\n"));
        assert!(request.contains("filename=\"speech.wav\"\r\nContent-Type: audio/wav"));
    }

    #[tokio::test]
    async fn refusals_are_closed_errors_and_never_retried() {
        let (base, seen) = serve(vec![
            http_reply(
                "402 Payment Required",
                "",
                "{\"error\":{\"code\":\"insufficient_credits\"}}",
            ),
            http_reply(
                "401 Unauthorized",
                "",
                "{\"error\":{\"code\":\"unauthorized\"}}",
            ),
            http_reply("429 Too Many Requests", "retry-after: 7\r\n", "{}"),
        ]);
        let target = at(&base);
        let wav = zephium_wav();
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::OverBudget)
        );
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::Unauthorized)
        );
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::RateLimited {
                retry_after_ms: Some(7_000)
            })
        );
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_oversized_take_is_refused_before_it_is_sent() {
        let mut target = at("http://127.0.0.1:9");
        target.max_bytes = 1_024;
        let wav = vec![0_u8; 2_048];
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::ContextTooLong)
        );
        target.max_bytes = PROVIDER_MAX_BYTES;
        assert_eq!(
            transcribe(&target, &credential(), &[], None).await,
            Err(WorkModelError::BadRequest)
        );
    }

    fn zephium_wav() -> Vec<u8> {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0; 40]);
        wav
    }
}
