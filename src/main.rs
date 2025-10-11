use mdbook::book::{Book, BookItem};
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use regex::Regex;
use anyhow::{bail, Result};
use std::process::Command;
use std::fs;
use std::path::PathBuf;
use std::io::Write;
use uuid::Uuid;

pub struct MermaidPreprocessor;

impl Preprocessor for MermaidPreprocessor {
    fn name(&self) -> &str {
        "mermaid-preprocessor"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book> {
        let mermaid_re = Regex::new(r"```mermaid\n([\s\S]*?)\n```")?;

        let output_dir = ctx.root.join("src").join("generated").join("diagrams");
        fs::remove_dir_all(&output_dir)?;
        fs::create_dir_all(&output_dir)?;

        for item in &mut book.sections {
            if let BookItem::Chapter(chapter) = item {
                let mut chapter_content = chapter.content.clone();
                let mut replacements: Vec<(String, String)> = Vec::new();

                for cap in mermaid_re.captures_iter(&chapter.content) {
                    let full_match = cap[0].to_string();
                    let mermaid_code = cap[1].to_string();

                    let uuid = Uuid::new_v4();
                    let svg_filename = format!("{}.svg", uuid);
                    let svg_filepath = output_dir.join(&svg_filename);
                    let relative_svg_path = PathBuf::from("generated").join("diagrams").join(&svg_filename);

                    let mut child = if cfg!(windows) {
                        Command::new("powershell")
                            .arg("-NoProfile")
                            .arg("-Command")
                            .arg("mmdc.ps1")
                            .arg("-i")
                            .arg("-")
                            .arg("-o")
                            .arg(&svg_filepath)
                            .stdin(std::process::Stdio::piped())
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::piped())
                            .spawn()?
                    } else {
                        Command::new("mmdc")
                            .arg("-i")
                            .arg("-")
                            .arg("-o")
                            .arg(&svg_filepath)
                            .stdin(std::process::Stdio::piped())
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::piped())
                            .spawn()?
                    };

                    if let Some(mut stdin) = child.stdin.take() {
                        stdin.write_all(mermaid_code.as_bytes())?;
                    }

                    let output = child.wait_with_output()?;

                    if !output.status.success() {
                        bail!("mmdc failed: {}\nStdout: {}\nStderr: {}",
                            output.status,
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr));
                    }

                    let img_tag = format!("![diagram](./{})", relative_svg_path.to_string_lossy());
                    replacements.push((full_match, img_tag));
                }

                for (old, new) in replacements {
                    chapter_content = chapter_content.replace(&old, &new);
                }
                chapter.content = chapter_content;
            }
        }

        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        true
    }
}

fn main() {
    mdbook_preprocessor_boilerplate::run(
        MermaidPreprocessor,
        "An mdbook preprocessor that renders mermaid diagrams."
    );
}