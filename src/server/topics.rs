//! Suggesting followed channels for a group.
//!
//! Each channel is turned into a vector by a small sentence-embedding model
//! (all-MiniLM-L6-v2, 6 layers, 384 numbers) from its name, description and
//! recent upload titles. A group is turned into one from its name and the
//! channels already in it, and follows are ranked by how close they sit.
//! Channels are the shared catalog, so each is embedded once for every
//! account, and again only when the model changes or a month has passed.
//!
//! The model is fetched from Hugging Face on first use, pinned to a revision
//! and checked against known hashes, into the data directory. It runs on the
//! CPU in a blocking thread. A test against the real model is `#[ignore]`d,
//! as the YouTube one is; the ranking itself is tested without it.

use super::*;
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig, DTYPE};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

/// Names the model and revision that made a stored vector. Vectors from any
/// other model are not comparable and are made again.
const MODEL_ID: &str = "all-MiniLM-L6-v2@1110a24";

const MODEL_REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";

const MODEL_REVISION: &str = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41";

/// Each file the model needs, with its SHA-256 at that revision.
const MODEL_FILES: [(&str, &str); 3] = [
    (
        "config.json",
        "953f9c0d463486b10a6871cc2fd59f223b2c70184f49815e7efbcab5d8908b41",
    ),
    (
        "tokenizer.json",
        "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037",
    ),
    (
        "model.safetensors",
        "53aa51172d142c89d9012cce15ae4d6cc0ca6895895114379cacb4fab128d9db",
    ),
];

/// The model was trained on 256 tokens; past that it only costs time.
const MAX_TOKENS: usize = 256;

/// Texts per forward pass. Each pass runs in one blocking task, so a long
/// first run still lets the runtime breathe between batches.
const EMBED_BATCH: usize = 32;

/// Recent titles say what a channel makes now, and the feed already has them.
const TITLES_PER_CHANNEL: usize = 15;

/// Descriptions are mostly links and sponsor copy after the first lines.
const DESCRIPTION_CHARS: usize = 400;

/// How far above the average follow's score a channel must be, in standard
/// deviations. Measured on a 724-channel account: 3 kept the two real
/// cooking channels and nothing else, while 2 still listed most of 40 tech
/// channels for a broad "Coding and tech".
const LIKELY_Z: f32 = 3.0;

const MAYBE_Z: f32 = 2.0;

/// A smaller account has fewer channels on any subject, so fewer clear 2:
/// on 60 follows, The Verge sat at 1.9 for "Tech". The best few that are at
/// least somewhat above average are always offered, unticked.
const MIN_MAYBE: usize = 10;

const FLOOR_Z: f32 = 1.0;

const MAX_SUGGESTIONS: usize = 40;

/// With too few follows the spread means nothing, so all are listed, unticked.
const MIN_FOR_SPREAD: usize = 8;

/// How much the group's name counts against the channels already in it.
const NAME_WEIGHT: f32 = 0.5;

static TOPIC_MODEL: tokio::sync::OnceCell<Arc<TopicModel>> = tokio::sync::OnceCell::const_new();

/// One embedding run at a time: two groups asked for at once on a fresh
/// server would otherwise embed every channel twice.
static EMBED_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) struct TopicModel {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl TopicModel {
    fn load(dir: &std::path::Path) -> Result<Self> {
        let device = Device::Cpu;
        let config: BertConfig = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|error| anyhow!("reading the tokenizer: {error}"))?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|error| anyhow!("setting up the tokenizer: {error}"))?;
        let weights = std::fs::read(dir.join("model.safetensors"))?;
        let model = BertModel::load(
            VarBuilder::from_buffered_safetensors(weights, DTYPE, &device)?,
            &config,
        )?;
        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// One unit-length vector per text: the mean of the token vectors,
    /// leaving out padding, as sentence-transformers pools this model.
    pub(crate) fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|error| anyhow!("tokenizing: {error}"))?;
        let ids = encodings
            .iter()
            .map(|encoding| Tensor::new(encoding.get_ids(), &self.device))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let masks = encodings
            .iter()
            .map(|encoding| Tensor::new(encoding.get_attention_mask(), &self.device))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let ids = Tensor::stack(&ids, 0)?;
        let mask = Tensor::stack(&masks, 0)?;
        let hidden = self.model.forward(&ids, &ids.zeros_like()?, Some(&mask))?;
        let mask = mask.to_dtype(DTYPE)?.unsqueeze(2)?;
        let mean = hidden
            .broadcast_mul(&mask)?
            .sum(1)?
            .broadcast_div(&mask.sum(1)?)?;
        let length = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
        Ok(mean.broadcast_div(&length)?.to_vec2::<f32>()?)
    }
}

/// Where the model's files live: `TAWNY_TOPIC_MODEL_DIR` for an offline copy,
/// otherwise under the data directory.
fn model_dir() -> std::path::PathBuf {
    std::env::var("TAWNY_TOPIC_MODEL_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| default_data_dir().join("models").join("all-MiniLM-L6-v2"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The model's files, fetched if missing or not what the revision holds.
async fn ensure_model_files() -> Result<std::path::PathBuf> {
    let dir = model_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    // Its own client: the shared one gives up after 15 seconds, and the
    // weights are 90 MB.
    let http = reqwest::Client::builder()
        .user_agent("Tawny/0.1 model download")
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()?;
    for (file, expected) in MODEL_FILES {
        let path = dir.join(file);
        let present = match tokio::fs::read(&path).await {
            Ok(bytes) => {
                tokio::task::spawn_blocking(move || sha256_hex(&bytes) == expected).await?
            }
            Err(_) => false,
        };
        if present {
            continue;
        }
        tracing::info!(file, "downloading the group suggestion model");
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REVISION}/{file}");
        let bytes = http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
            .with_context(|| format!("downloading {file}"))?;
        let bytes = bytes.to_vec();
        let (bytes, actual) = tokio::task::spawn_blocking(move || {
            let actual = sha256_hex(&bytes);
            (bytes, actual)
        })
        .await?;
        if actual != expected {
            return Err(anyhow!(
                "{file} from Hugging Face did not match its expected hash"
            ));
        }
        // Written aside and moved, so an interrupted download is never
        // taken for a finished one.
        let partial = dir.join(format!("{file}.partial"));
        tokio::fs::write(&partial, &bytes).await?;
        tokio::fs::rename(&partial, &path).await?;
    }
    Ok(dir)
}

/// The loaded model, fetched and loaded on first use. A failure is not
/// remembered, so the next request tries again.
async fn topic_model() -> Result<Arc<TopicModel>> {
    TOPIC_MODEL
        .get_or_try_init(|| async {
            let dir = ensure_model_files().await?;
            let model = tokio::task::spawn_blocking(move || TopicModel::load(&dir)).await??;
            Ok::<_, anyhow::Error>(Arc::new(model))
        })
        .await
        .cloned()
}

async fn embed(model: &Arc<TopicModel>, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
    let model = model.clone();
    tokio::task::spawn_blocking(move || model.embed(&texts)).await?
}

/// What the model reads for one channel.
fn channel_text(name: &str, description: &str, titles: &[String]) -> String {
    let description = description
        .split_whitespace()
        .filter(|word| !looks_like_link(word))
        .collect::<Vec<_>>()
        .join(" ");
    let description = description
        .char_indices()
        .nth(DESCRIPTION_CHARS)
        .map_or(description.as_str(), |(end, _)| &description[..end]);
    let titles = titles
        .iter()
        .take(TITLES_PER_CHANNEL)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("; ");
    format!("{name}. {description}. Videos: {titles}")
}

/// Links and addresses say nothing about a channel's subject, and a
/// description full of them otherwise reads as "a channel with links".
fn looks_like_link(word: &str) -> bool {
    let word = word.to_ascii_lowercase();
    word.contains("://") || word.starts_with("www.") || (word.contains('@') && word.contains('.'))
}

fn unit(mut vector: Vec<f32>) -> Vec<f32> {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        vector.iter_mut().for_each(|value| *value /= length);
    }
    vector
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

/// Rank `candidates` against a group: its name's vector and those of the
/// channels already in it.
///
/// Scores are judged against the other follows rather than a fixed bar:
/// how close a match is depends on the group's name and on how many
/// similar channels the account follows, but a real match stands out from
/// that account's average either way.
fn rank_suggestions(
    name: &[f32],
    members: &[&[f32]],
    candidates: &[(&str, &[f32])],
) -> Vec<GroupSuggestion> {
    let query = if members.is_empty() {
        name.to_vec()
    } else {
        let mut centroid = vec![0.0; name.len()];
        for member in members {
            centroid
                .iter_mut()
                .zip(member.iter())
                .for_each(|(sum, value)| *sum += value);
        }
        let centroid = unit(centroid);
        unit(
            name.iter()
                .zip(&centroid)
                .map(|(name, centroid)| NAME_WEIGHT * name + (1.0 - NAME_WEIGHT) * centroid)
                .collect(),
        )
    };
    let mut scored = candidates
        .iter()
        .map(|(id, vector)| (*id, dot(&query, vector)))
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    if scored.len() < MIN_FOR_SPREAD {
        return scored
            .into_iter()
            .map(|(id, _)| GroupSuggestion {
                channel_id: id.to_string(),
                likely: false,
            })
            .collect();
    }
    let count = scored.len() as f32;
    let mean = scored.iter().map(|(_, score)| score).sum::<f32>() / count;
    let spread = (scored
        .iter()
        .map(|(_, score)| (score - mean).powi(2))
        .sum::<f32>()
        / count)
        .sqrt();
    if spread <= f32::EPSILON {
        return Vec::new();
    }
    scored
        .into_iter()
        .map(|(id, score)| (id, (score - mean) / spread))
        .enumerate()
        .take_while(|(rank, (_, z))| *z >= MAYBE_Z || (*rank < MIN_MAYBE && *z >= FLOOR_Z))
        .take(MAX_SUGGESTIONS)
        .map(|(_, (id, z))| GroupSuggestion {
            channel_id: id.to_string(),
            likely: z >= LIKELY_Z,
        })
        .collect()
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbTopicVector {
    channel_id: String,
    topic_vector: Option<Vec<f32>>,
    fresh: bool,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbTopicSource {
    channel_id: String,
    name: String,
    description: String,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbRecentTitles {
    channel_id: String,
    titles: Vec<String>,
}

#[derive(Debug, Serialize, SurrealValue)]
struct TopicWrite {
    channel_id: String,
    vector: Vec<f32>,
}

impl AppServerState {
    /// Followed channels that look like they belong in a group, best first.
    pub async fn suggest_group_channels(
        &self,
        owner: &str,
        group_id: &str,
    ) -> Result<Vec<GroupSuggestion>> {
        let groups: Vec<DbSubscriptionGroup> = self
            .db
            .query("SELECT group_id, name, channel_ids FROM subscription_group WHERE owner = type::record($owner) AND group_id = $group_id LIMIT 1")
            .bind(("owner", owner.to_string()))
            .bind(("group_id", group_id.to_string()))
            .await?
            .check()?
            .take(0)?;
        let group = SubscriptionGroup::from(
            groups
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("that group no longer exists"))?,
        );
        let followed = self
            .user_subscriptions(owner)
            .await?
            .into_iter()
            .filter(|(_, (subscribed, _))| *subscribed)
            .map(|(channel_id, _)| channel_id)
            .collect::<Vec<_>>();
        if followed.is_empty() {
            return Ok(Vec::new());
        }

        let model = topic_model()
            .await
            .context("the group suggestion model could not be loaded")?;
        let vectors = self.topic_vectors(&model, &followed).await?;
        let name = embed(&model, vec![group.name.clone()])
            .await?
            .pop()
            .ok_or_else(|| anyhow!("the model returned nothing for the group name"))?;

        let members = group
            .channel_ids
            .iter()
            .filter_map(|id| vectors.get(id).map(Vec::as_slice))
            .collect::<Vec<_>>();
        let in_group = group.channel_ids.iter().collect::<HashSet<_>>();
        let candidates = followed
            .iter()
            .filter(|id| !in_group.contains(id))
            .filter_map(|id| {
                vectors
                    .get(id)
                    .map(|vector| (id.as_str(), vector.as_slice()))
            })
            .collect::<Vec<_>>();
        Ok(rank_suggestions(&name, &members, &candidates))
    }

    /// Each channel's vector, embedding those without a current one first.
    async fn topic_vectors(
        &self,
        model: &Arc<TopicModel>,
        channel_ids: &[String],
    ) -> Result<HashMap<String, Vec<f32>>> {
        let _guard = EMBED_LOCK.lock().await;
        let rows: Vec<DbTopicVector> = self
            .db
            .query("SELECT channel_id, topic_vector, (topic_model = $model AND topic_at != NONE AND topic_at > time::now() - 30d) AS fresh FROM channel WHERE channel_id IN $ids")
            .bind(("model", MODEL_ID))
            .bind(("ids", channel_ids.to_vec()))
            .await?
            .check()?
            .take(0)?;
        let mut vectors = HashMap::new();
        let mut stale = Vec::new();
        for row in rows {
            match row.topic_vector {
                Some(vector) if row.fresh => {
                    vectors.insert(row.channel_id, vector);
                }
                _ => stale.push(row.channel_id),
            }
        }
        if stale.is_empty() {
            return Ok(vectors);
        }

        tracing::info!(
            channels = stale.len(),
            "embedding channels for group suggestions"
        );
        // The per-channel title lookup walks the (channel_id, published_sort)
        // index backwards; one query over all the channels' videos at once
        // took 20 times as long on 724 follows.
        let mut response = self
            .db
            .query("SELECT channel_id, name, description FROM channel WHERE channel_id IN $ids")
            .query(format!("RETURN array::map($ids, |$id| {{ channel_id: $id, titles: (SELECT VALUE title FROM video WHERE channel_id = $id ORDER BY published_sort DESC LIMIT {TITLES_PER_CHANNEL}) }})"))
            .bind(("ids", stale))
            .await?
            .check()?;
        let sources: Vec<DbTopicSource> = response.take(0)?;
        let titles: Vec<DbRecentTitles> = response.take(1)?;
        let mut titles = titles
            .into_iter()
            .map(|row| (row.channel_id, row.titles))
            .collect::<HashMap<_, _>>();

        for batch in sources.chunks(EMBED_BATCH) {
            let texts = batch
                .iter()
                .map(|source| {
                    let titles = titles.remove(&source.channel_id).unwrap_or_default();
                    channel_text(&source.name, &source.description, &titles)
                })
                .collect::<Vec<_>>();
            let embedded = embed(model, texts).await?;
            let writes = batch
                .iter()
                .zip(embedded)
                .map(|(source, vector)| {
                    vectors.insert(source.channel_id.clone(), vector.clone());
                    TopicWrite {
                        channel_id: source.channel_id.clone(),
                        vector,
                    }
                })
                .collect::<Vec<_>>();
            self.db
                .query("FOR $row IN $rows { UPDATE type::record('channel', $row.channel_id) SET topic_vector = $row.vector, topic_model = $model, topic_at = time::now(); }")
                .bind(("model", MODEL_ID))
                .bind(("rows", writes))
                .await?
                .check()?;
        }
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit vector mostly along `axis`, nudged by `noise` along the next.
    fn toward(axis: usize, noise: f32) -> Vec<f32> {
        let mut vector = vec![0.0; 8];
        vector[axis] = 1.0;
        vector[(axis + 1) % 8] = noise;
        unit(vector)
    }

    fn ids(suggestions: &[GroupSuggestion]) -> Vec<&str> {
        suggestions
            .iter()
            .map(|suggestion| suggestion.channel_id.as_str())
            .collect()
    }

    #[test]
    fn a_clear_match_stands_out_from_the_other_follows() {
        let names = (0..30)
            .map(|index| format!("other-{index}"))
            .collect::<Vec<_>>();
        let others = (0..30)
            .map(|index| toward(1 + index % 6, 0.1 * (index % 3) as f32))
            .collect::<Vec<_>>();
        let tech = toward(0, 0.05);
        let mut candidates = vec![("tech", tech.as_slice())];
        candidates.extend(
            names
                .iter()
                .zip(&others)
                .map(|(name, vector)| (name.as_str(), vector.as_slice())),
        );
        let suggestions = rank_suggestions(&toward(0, 0.0), &[], &candidates);
        assert_eq!(ids(&suggestions), vec!["tech"]);
        assert!(suggestions[0].likely);
    }

    #[test]
    fn without_a_standout_only_the_best_few_are_offered_unticked() {
        // Scores spread evenly from -1 to 1: the best is only about 1.7
        // deviations above the average, short of both bars.
        let vectors = (0..21)
            .map(|index| {
                let score = index as f32 / 10.0 - 1.0;
                let mut vector = vec![0.0; 8];
                vector[0] = score;
                vector[1] = (1.0 - score * score).max(0.0).sqrt();
                vector
            })
            .collect::<Vec<_>>();
        let names = (0..21).map(|index| index.to_string()).collect::<Vec<_>>();
        let candidates = names
            .iter()
            .zip(&vectors)
            .map(|(name, vector)| (name.as_str(), vector.as_slice()))
            .collect::<Vec<_>>();
        let suggestions = rank_suggestions(&toward(0, 0.0), &[], &candidates);
        // 1.0, 0.9, 0.8 and 0.7 are a deviation or more above the average.
        assert_eq!(ids(&suggestions), vec!["20", "19", "18", "17"]);
        assert!(suggestions.iter().all(|suggestion| !suggestion.likely));
        // Identical scores offer nothing at all.
        let same = toward(0, 0.0);
        let flat = names
            .iter()
            .map(|name| (name.as_str(), same.as_slice()))
            .collect::<Vec<_>>();
        assert!(rank_suggestions(&toward(1, 0.0), &[], &flat).is_empty());
    }

    #[test]
    fn channels_already_in_the_group_steer_a_vague_name() {
        // "Chill" says little; the group's members say "axis 3".
        let vague = unit(vec![1.0; 8]);
        let member = toward(3, 0.0);
        let near_members = toward(3, 0.1);
        let others = (0..12)
            .map(|index| toward(index % 3, 0.2))
            .collect::<Vec<_>>();
        let names = (0..12).map(|index| index.to_string()).collect::<Vec<_>>();
        let mut candidates = vec![("near", near_members.as_slice())];
        candidates.extend(
            names
                .iter()
                .zip(&others)
                .map(|(name, vector)| (name.as_str(), vector.as_slice())),
        );
        let with_members = rank_suggestions(&vague, &[member.as_slice()], &candidates);
        assert_eq!(ids(&with_members).first(), Some(&"near"));
    }

    #[test]
    fn a_few_follows_are_all_offered_unticked() {
        let a = toward(0, 0.0);
        let b = toward(1, 0.0);
        let suggestions = rank_suggestions(
            &toward(1, 0.0),
            &[],
            &[("a", a.as_slice()), ("b", b.as_slice())],
        );
        assert_eq!(ids(&suggestions), vec!["b", "a"]);
        assert!(suggestions.iter().all(|suggestion| !suggestion.likely));
    }

    #[test]
    fn the_model_reads_the_subject_not_the_links() {
        let text = channel_text(
            "Code to the Moon",
            "Rust and systems programming. https://patreon.com/x www.example.com hi@example.com @handle",
            &[
                "Rust Atomics Demystified".into(),
                "Lean vs $12 server".into(),
            ],
        );
        assert_eq!(
            text,
            "Code to the Moon. Rust and systems programming. @handle. Videos: Rust Atomics Demystified; Lean vs $12 server"
        );
        let long = "word ".repeat(200);
        let text = channel_text("Long", &long, &[]);
        assert!(text.len() < DESCRIPTION_CHARS + 30, "{}", text.len());
    }

    #[tokio::test]
    async fn several_channels_join_a_group_in_one_write() {
        let state = AppServerState::initialize().await.unwrap();
        let owner = state.accounts().create_guest().await.unwrap().id;
        let group = SubscriptionGroup {
            id: "tech".into(),
            name: "Tech".into(),
            channel_ids: vec!["a".into()],
        };
        state.save_groups(&owner, &[group]).await.unwrap();
        let added = state
            .set_many_in_group(&owner, "tech", &["a".into(), "b".into(), "c".into()], true)
            .await
            .unwrap();
        assert!(added);
        // Set, not toggled: sending it again changes nothing.
        let again = state
            .set_many_in_group(&owner, "tech", &["b".into(), "c".into()], true)
            .await
            .unwrap();
        assert!(!again);
        let viewer = state.viewer(&owner).await.unwrap();
        let tech = viewer
            .subscription_groups
            .iter()
            .find(|group| group.id == "tech")
            .unwrap();
        assert_eq!(tech.channel_ids, vec!["a", "b", "c"]);
    }

    /// Downloads the real model (90 MB) and checks it sorts by subject.
    #[tokio::test]
    #[ignore = "downloads the model from Hugging Face"]
    async fn the_real_model_sorts_channels_by_subject() {
        let model = topic_model().await.unwrap();
        let texts = vec![
            "Coding and tech".to_string(),
            channel_text(
                "Code to the Moon",
                "Rust and systems programming.",
                &["Rust Atomics Demystified".into()],
            ),
            channel_text(
                "JunsKitchen",
                "Cooking with my cats in Japan.",
                &["Angel hair shrimp".into()],
            ),
        ];
        let vectors = embed(&model, texts).await.unwrap();
        assert_eq!(vectors[0].len(), 384);
        assert!((dot(&vectors[0], &vectors[0]) - 1.0).abs() < 1e-4);
        assert!(dot(&vectors[0], &vectors[1]) > dot(&vectors[0], &vectors[2]));
    }
}
