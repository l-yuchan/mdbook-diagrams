use anyhow::{Result, bail};
use mdbook::BookItem;
use mdbook::book::{Book, Chapter};
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use regex::Regex;
use sha2::{Sha256, Digest};
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

/// Rendering mode for diagrams
#[derive(Debug, Clone, Copy, PartialEq)]
enum RenderMode {
    /// Pre-render diagrams at build time (default)
    PreRender,
    /// Embed mermaid code and render at runtime in browser
    Runtime,
}

/// Represents an edit to be applied to a chapter's content.
struct ChapterEdit {
    chapter_path: PathBuf,
    range: Range<usize>, // The byte range of the original mermaid block
    new_string: String,
    cached_filename: String, // The cache filename that was used or created
}

pub struct DiagramsPreprocessor {
    render_mode: RenderMode,
    mmdc_cmd: String,
    output_format: String,
    enable_cache: bool,
}

impl DiagramsPreprocessor {
    pub fn new(config: Option<Table>) -> DiagramsPreprocessor {
        let render_mode = config
            .as_ref()
            .and_then(|table| table.get("render-mode"))
            .and_then(|val| val.as_str())
            .map(|s| match s {
                "runtime" => RenderMode::Runtime,
                "pre-render" => RenderMode::PreRender,
                _ => {
                    eprintln!("[mdbook-diagrams] Invalid render-mode: {}, falling back to pre-render", s);
                    eprintln!("[mdbook-diagrams] Available modes: runtime, pre-render");
                    RenderMode::PreRender
                },
            })
            .unwrap_or(RenderMode::PreRender);

        let mmdc_cmd = config
            .as_ref()
            .and_then(|table| table.get("mmdc-cmd"))
            .and_then(|val| val.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "mmdc".to_string());

        let output_format = config
            .as_ref()
            .and_then(|table| table.get("output-format"))
            .and_then(|val| val.as_str())
            .map(|s| match s {
                "svg" => "svg".to_string(),
                "png" => "png".to_string(),
                _ => {
                    eprintln!("[mdbook-diagrams] Invalid output-format: {}, falling back to svg", s);
                    eprintln!("[mdbook-diagrams] Available formats: svg, png");
                    "svg".to_string()
                },
            })
            .unwrap_or_else(|| "svg".to_string());

        let enable_cache = config
            .as_ref()
            .and_then(|table| table.get("enable-cache"))
            .and_then(|val| val.as_bool())
            .unwrap_or(true);

        DiagramsPreprocessor {
            render_mode,
            mmdc_cmd,
            output_format,
            enable_cache,
        }
    }

    /// Compute a cache hash from diagram content and rendering configuration
    fn compute_cache_hash(content: &str, output_format: &str, mmdc_cmd: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hasher.update(output_format.as_bytes());
        hasher.update(mmdc_cmd.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)
    }

    fn prepare_mermaid_files(&self, ctx: &PreprocessorContext) -> Result<()> {
        let theme_dir = ctx.root.join("theme");
        std::fs::create_dir_all(&theme_dir)?;

        let mermaid_js_path = theme_dir.join("mermaid.min.js");
        let mermaid_init_path = theme_dir.join("mermaid-init.js");

        let mut js_updated = false;

        // Download mermaid.min.js if it doesn't exist
        if !mermaid_js_path.exists() {
            eprintln!("Downloading mermaid.min.js...");
            let url = "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js";
            let response = reqwest::blocking::get(url)?;
            let content = response.bytes()?;
            std::fs::write(&mermaid_js_path, content)?;
            js_updated = true;
            eprintln!("Downloaded mermaid.min.js to theme/mermaid.min.js");
        }

        // Create mermaid-init.js if it doesn't exist
        if !mermaid_init_path.exists() {
            let init_script = r#"import mermaid from './mermaid.min.js';
mermaid.initialize({ startOnLoad: true });
"#;
            std::fs::write(&mermaid_init_path, init_script)?;
            js_updated = true;
            eprintln!("Created mermaid-init.js at theme/mermaid-init.js");
        }

        if js_updated {
            eprintln!("[mdbook-diagrams] mermaid.min.js and mermaid-init.js is created in theme/ directory.");
            eprintln!("[mdbook-diagrams] To enable runtime rendering, please add the following to your book.toml:\n");
            eprintln!("[output.html]");
            eprintln!("additional-js = [\"theme/mermaid.min.js\", \"theme/mermaid-init.js\"]");
        }

        Ok(())
    }

    fn process_book_for_runtime_mode(&self, mut book: Book) -> Result<Book> {
        let mermaid_re = Regex::new(r#"```mermaid\r?\n([\s\S]*?)\r?\n```"#)?;

        for item in &mut book.sections {
            Self::process_book_item_for_runtime_mode(&mermaid_re, item);
        }

        Ok(book)
    }

    /// Recursively process book items to convert mermaid blocks to HTML `pre` tags
    fn process_book_item_for_runtime_mode(mermaid_re: &Regex, book_item: &mut BookItem) {
        if let BookItem::Chapter(chapter) = book_item {
            chapter.content = mermaid_re.replace_all(&chapter.content, |caps: &regex::Captures| {
                let diagram_code = &caps[1];
                format!("<pre class=\"mermaid\">\n{}\n</pre>", diagram_code)
            }).to_string();

            for sub_item in &mut chapter.sub_items {
                Self::process_book_item_for_runtime_mode(mermaid_re, sub_item);
            }
        }
    }

    async fn async_process_book(&self, ctx: &PreprocessorContext, book: &mut Book) -> Result<()> {
        let mermaid_re = Regex::new(r#"```mermaid\r?\n([\s\S]*?)\r?\n```"#)?;

        let output_dir = ctx.root.join(&ctx.config.book.src).join("generated").join("diagrams");
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

        let edits: Vec<ChapterEdit> = futures::future::join_all(all_futures).await.into_iter()
            .filter_map(|e| match e {
                Ok(e) => Some(e),
                Err(e) => {
                    eprintln!("[mdbook-diagrams] Failed to generate diagram: {}", e);
                    None
                }
            }
        ).collect();

        // Extract referenced filenames for cleanup
        let referenced_files: std::collections::HashSet<String> = edits
            .iter()
            .map(|edit| edit.cached_filename.clone())
            .filter(|name| !name.is_empty())
            .collect();

        // Clean up unreferenced cache files if caching is enabled
        if self.enable_cache {
            if let Err(e) = Self::cleanup_unreferenced_files(&output_dir, &referenced_files).await {
                eprintln!("[mdbook-diagrams] Warning: Failed to clean up cache files: {}", e);
            }
        }

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
                sorted_edits.reverse();

                for edit in sorted_edits {
                    // Replace the content using the byte range
                    chapter.content.replace_range(edit.range, &edit.new_string);
                }
            }

            // Recursively apply to sub items
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

    /// Generate diagrams for all mermaid blocks in a chapter and return a list of edits to apply.
    fn collect_edits_from_chapter(
        &'_ self,
        mermaid_re: & Regex,
        chapter: & Chapter,
        output_dir: & PathBuf,
        semaphore: & Arc<tokio::sync::Semaphore>,
    ) -> Vec<BoxFuture<'_, Result<ChapterEdit>>> {
        let mut futures = Vec::new();

        for cap in mermaid_re.captures_iter(&chapter.content) {
            let full_match_range = cap.get(0).unwrap().range();
            let mermaid_code = cap[1].to_string();
            let original_block = cap.get(0).unwrap().as_str().to_string();

            let cache_hash = Self::compute_cache_hash(&mermaid_code, &self.output_format, &self.mmdc_cmd);
            let output_filename = format!("{}.{}", cache_hash, self.output_format);
            let output_filepath = output_dir.join(&output_filename);

            let chapter_path = chapter.path.clone().unwrap_or_default();

            let relative_output_path = {
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
                path.push(&output_filename);
                path
            };

            let semaphore_clone = semaphore.clone();
            let mmdc_cmd = self.mmdc_cmd.clone();
            let enable_cache = self.enable_cache;

            futures.push(async move {
                if enable_cache && output_filepath.exists() {
                    // Cache hit - skip mmdc execution
                    let img_tag = format!(
                        "![diagram](./{})",
                        relative_output_path.to_string_lossy().replace("\\", "/")
                    );
                    return Ok(ChapterEdit {
                        chapter_path,
                        range: full_match_range,
                        new_string: img_tag,
                        cached_filename: output_filename.clone(),
                    });
                }

                // Cache miss or caching disabled - generate diagram
                let result = async {
                    let _permit = semaphore_clone.acquire().await?;
                    let mut command = if cfg!(windows) {
                        let mut cmd = tokio::process::Command::new("powershell");
                        cmd.arg("-NoProfile")
                            .arg("-Command")
                            .arg(&mmdc_cmd)
                            .arg("-i")
                            .arg("-")
                            .arg("-o")
                            .arg(&output_filepath)
                            .stdin(std::process::Stdio::piped())
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::piped());
                        cmd
                    } else {
                        let mut cmd = tokio::process::Command::new(&mmdc_cmd);
                        cmd.arg("-i")
                            .arg("-")
                            .arg("-o")
                            .arg(&output_filepath)
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
                            "mmdc failed: {}\nStderr: {}",
                            output.status,
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }

                    Ok::<String, anyhow::Error>(format!(
                        "![diagram](./{})",
                        relative_output_path.to_string_lossy().replace("\\", "/")
                    ))
                }.await;

                // Handle result - on error, keep the original mermaid block with an error message
                match result {
                    Ok(img_tag) => Ok(ChapterEdit {
                        chapter_path,
                        range: full_match_range,
                        new_string: img_tag,
                        cached_filename: output_filename.clone(),
                    }),
                    Err(e) => {
                        let error_msg = format!("{:#}", e);
                        eprintln!("[mdbook-diagrams] {}", error_msg);

                        // Keep original mermaid block with error comment
                        let error_comment = format!(
                            "<!-- Error generating diagram: {} -->\n{}",
                            error_msg.lines().next().unwrap_or("Unknown error"),
                            original_block
                        );
                        Ok(ChapterEdit {
                            chapter_path,
                            range: full_match_range,
                            new_string: error_comment,
                            cached_filename: String::new(),
                        })
                    }
                }
            }.boxed())
        }
        futures
    }

    /// Remove cache files that are not referenced in the current build
    async fn cleanup_unreferenced_files(
        output_dir: &PathBuf,
        referenced_files: &std::collections::HashSet<String>,
    ) -> Result<()> {
        let mut entries = tokio::fs::read_dir(output_dir).await?;
        let mut removed_count = 0;

        while let Some(entry) = entries.next_entry().await? {
            if let Ok(filename) = entry.file_name().into_string() {
                if !referenced_files.contains(&filename) && !filename.is_empty() {
                    if let Err(e) = tokio::fs::remove_file(entry.path()).await {
                        eprintln!(
                            "[mdbook-diagrams] Warning: Failed to remove unreferenced cache file {}: {}",
                            filename, e
                        );
                    } else {
                        removed_count += 1;
                    }
                }
            }
        }

        Ok(())
    }
}

impl Preprocessor for DiagramsPreprocessor {
    fn name(&self) -> &str {
        "mdbook-diagrams"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book> {
        match self.render_mode {
            RenderMode::Runtime => {
                self.prepare_mermaid_files(ctx)?;
                book = self.process_book_for_runtime_mode(book)?;
            }
            RenderMode::PreRender => {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()?;

                runtime.block_on(self.async_process_book(ctx, &mut book))?;
            }
        }

        Ok(book)
    }
}
