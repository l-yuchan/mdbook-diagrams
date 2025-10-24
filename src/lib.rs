use anyhow::{Result, bail};
use mdbook::BookItem;
use mdbook::book::{Book, Chapter};
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use regex::Regex;
use std::collections::HashMap;
use std::num::NonZero;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use futures::future::BoxFuture;
use futures::FutureExt;
use tokio::io::AsyncWriteExt;
use toml::value::Table;
use uuid::Uuid;

/// Represents an edit to be applied to a chapter's content.
struct ChapterEdit {
    chapter_path: PathBuf,
    range: Range<usize>, // The byte range of the original mermaid block
    new_string: String,
}

pub struct DiagramsPreprocessor {
    config: Option<Table>,
}

impl DiagramsPreprocessor {
    pub fn new(config: Option<Table>) -> DiagramsPreprocessor {
        DiagramsPreprocessor { config }
    }

    async fn async_process_book(&self, ctx: &PreprocessorContext, book: &mut Book) -> Result<()> {
        let mermaid_re = Regex::new(r#"```mermaid\r?\n([\s\S]*?)\r?\n```"#)?;

        let output_dir = ctx.root.join(&ctx.config.book.src).join("generated").join("diagrams");
        if output_dir.exists() {
            tokio::fs::remove_dir_all(&output_dir).await?;
        }
        tokio::fs::create_dir_all(&output_dir).await?;

        let num_cpus = std::thread::available_parallelism()
            .unwrap_or(NonZero::new(1).unwrap())
            .get();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(num_cpus));

        // Collect all futures from book items
        let mut all_futures = Vec::new();
        for item in &mut book.sections {
            all_futures.extend(self.collect_edits_from_book_item_recursively(
                &mermaid_re,
                item,
                &output_dir,
                &semaphore,
            ));
        }

        let edits: Vec<ChapterEdit> = futures::future::join_all(all_futures).await.into_iter().filter_map(|e| e.ok()).collect();

        // Group edits by chapter path for easier processing
        let mut edits_by_chapter: HashMap<PathBuf, Vec<ChapterEdit>> = HashMap::new();
        for edit in edits {
            edits_by_chapter.entry(edit.chapter_path.clone()).or_insert_with(Vec::new).push(edit);
        }

        // Iterate through the book mutably and apply edits recursively
        for item in &mut book.sections {
            DiagramsPreprocessor::apply_edits_to_book_item_recursively(item, &mut edits_by_chapter);
        }

        Ok(())
    }

    fn apply_edits_to_book_item_recursively(book_item: &mut BookItem, edits_by_chapter: &mut HashMap<PathBuf, Vec<ChapterEdit>>) {
        if let BookItem::Chapter(chapter) = book_item {
            let chapter_path = chapter.path.clone().unwrap_or_default();
            if let Some(chapter_edits) = edits_by_chapter.remove(&chapter_path) {
                // Sort edits by range start in descending order to avoid offset issues
                let mut sorted_edits = chapter_edits;
                sorted_edits.sort_by_key(|e| e.range.start);
                sorted_edits.reverse(); // Apply from end to beginning

                for edit in sorted_edits {
                    // Replace the content using the byte range
                    chapter.content.replace_range(edit.range, &edit.new_string);
                }
            }

            // Recursively apply to sub-items
            for sub_item in &mut chapter.sub_items {
                DiagramsPreprocessor::apply_edits_to_book_item_recursively(sub_item, edits_by_chapter);
            }
        }
    }

    fn collect_edits_from_book_item_recursively(
        &'_ self,
        mermaid_re: & Regex,
        book_item: & BookItem,
        output_dir: & PathBuf,
        semaphore: & Arc<tokio::sync::Semaphore>,
    ) -> Vec<BoxFuture<'_, Result<ChapterEdit>>> {
        let mut futures = Vec::new();
        if let BookItem::Chapter(chapter) = book_item {
            // Collect edits from a chapter
            futures.extend(
                self.collect_edits_from_chapter(mermaid_re, chapter, output_dir, semaphore),
            );

            // Proceed recursively for sub items
            for sub_item in &chapter.sub_items {
                futures.extend(self.collect_edits_from_book_item_recursively(
                    &mermaid_re, sub_item, &output_dir, &semaphore,
                ));
            }
        }
        futures
    }

    /// Generate SVGs for all mermaid blocks in a chapter and return a list of edits to apply.
    fn collect_edits_from_chapter(
        &'_ self,
        mermaid_re: & Regex,
        chapter: & Chapter,
        output_dir: & PathBuf,
        semaphore: & Arc<tokio::sync::Semaphore>,
    ) -> Vec<BoxFuture<'_, Result<ChapterEdit>>> {
        let mut futures = Vec::new();

        let mmdc_cmd = self.config
            .as_ref()
            .and_then(|table| table.get("mmdc_cmd"))
            .and_then(|val| val.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "mmdc".to_string());

        for cap in mermaid_re.captures_iter(&chapter.content) {
            let full_match_range = cap.get(0).unwrap().range();
            let mermaid_code = cap[1].to_string();

            let uuid = Uuid::new_v4();
            let svg_filename = format!("{}.svg", uuid);
            let svg_filepath = output_dir.join(&svg_filename);

            let chapter_path = chapter.path.clone().unwrap_or_default();

            let relative_svg_path = {
                let chapter_dir_relative_to_src = chapter
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .unwrap_or_else(|| Path::new(""));
                let num_parent_dirs = chapter_dir_relative_to_src.components().count();

                let mut path = PathBuf::new();
                for _ in 0..num_parent_dirs {
                    path.push("..");
                }
                path.push("generated");
                path.push("diagrams");
                path.push(&svg_filename);
                path
            };

            let semaphore_clone = semaphore.clone();
            let mmdc_cmd_clone = mmdc_cmd.clone();

            futures.push(async move {
                let _permit = semaphore_clone.acquire().await?;
                let mut command = if cfg!(windows) {
                    let mut cmd = tokio::process::Command::new("powershell");
                    cmd.arg("-NoProfile")
                        .arg("-Command")
                        .arg(&mmdc_cmd_clone)
                        .arg("-i")
                        .arg("-")
                        .arg("-o")
                        .arg(&svg_filepath)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped());
                    cmd
                } else {
                    let mut cmd = tokio::process::Command::new(&mmdc_cmd_clone);
                    cmd.arg("-i")
                        .arg("-")
                        .arg("-o")
                        .arg(&svg_filepath)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped());
                    cmd
                };

                let mut child = command.spawn()?;

                if let Some(mut stdin) = child.stdin.take() {
                    AsyncWriteExt::write_all(&mut stdin, mermaid_code.as_bytes()).await?;
                }

                let output = child.wait_with_output().await?;

                if !output.status.success() {
                    bail!(
                        "mmdc failed: {}\nStdout: {}\nStderr: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }

                let img_tag = format!(
                    "![diagram](./{})",
                    relative_svg_path.to_string_lossy().replace("\\", "/")
                );
                Ok(ChapterEdit {
                    chapter_path,
                    range: full_match_range,
                    new_string: img_tag,
                })
            }.boxed())
        }
        futures
    }
}

impl Preprocessor for DiagramsPreprocessor {
    fn name(&self) -> &str {
        "mdbook-diagrams"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        runtime.block_on(self.async_process_book(ctx, &mut book))?;

        Ok(book)
    }
}
