[![Crates.io](https://img.shields.io/crates/v/mdbook-diagrams)](https://crates.io/crates/mdbook-diagrams)
```
                _ _                 _         
               | | |               | |        
  _ __ ___   __| | |__   ___   ___ | | __     
 | '_ ` _ \ / _` | '_ \ / _ \ / _ \| |/ /     
 | | | | | | (_| | |_) | (_) | (_) |   <      
 |_| |_|_|_|\__,_|_.__/ \___/ \___/|_|\_\     
     | (_)                                    
   __| |_  __ _  __ _ _ __ __ _ _ __ ___  ___ 
  / _` | |/ _` |/ _` | '__/ _` | '_ ` _ \/ __|
 | (_| | | (_| | (_| | | | (_| | | | | | \__ \
  \__,_|_|\__,_|\__, |_|  \__,_|_| |_| |_|___/
                 __/ |                        
                |___/                         
```

`mdbook-diagrams` is a [mdbook](https://rust-lang.github.io/mdBook/) preprocessor for embedding [mermaidjs](https://mermaid.js.org/) diagrams.

Diagrams are generated at build time and saved at `src/generated/diagrams`. No need to include ~2.5MB js files! (You can also use runtime rendering if you prefer)

## Requirements

- mermaid-cli >= 11.12.0

## Usage

First, ensure you have installed mermaid-cli and have `mmdc` in your env PATH
```bash
npm install -g @mermaid-js/mermaid-cli
mmdc --version
```

...and run cargo command to install from source
```bash
cargo install mdbook-diagrams
```

After the installation, you can add this line to your `book.toml` to enable the preprocessor
```toml
[preprocessor.diagrams]
```

And that's it! Build your mdbook and see your diagrams embedded.

## Configuration

`mdbook-diagrams` supports several configuration options in your `book.toml`:

```toml
[preprocessor.diagrams]
# Enable or disable diagram caching (default: true)
# When enabled, diagrams are only regenerated when their content or configuration changes
enable-cache = true

# Output format for diagrams (default: "svg")
# Options: "svg" or "png"
output-format = "svg"

# Command to run mermaid-cli (default: "mmdc")
# Useful if mmdc is not in PATH or you want to use a specific version
mmdc-cmd = "mmdc"

# Rendering mode (default: "pre-render")
# Options: "pre-render" (generate at build time) or "runtime" (you should include mermaidJS files, render in browser)
render-mode = "pre-render"
```

### Caching

By default, `mdbook-diagrams` caches generated diagrams to significantly speed up builds. The cache:

- **Content-aware**: Diagrams are only regenerated when their source code changes
- **Configuration-aware**: Changing `output-format` or `mmdc-cmd` triggers regeneration

To disable caching (e.g., for debugging), set `enable-cache = false`.

### Runtime Rendering

If you prefer to render diagrams in the browser instead of at build time, set `render-mode = "runtime"`. This will:
- Skip pre-rendering with mermaid-cli
- Download `mermaid.min.js` to your `theme/` directory
- Embed mermaid code blocks for client-side rendering

Note: Runtime mode requires adding mermaid scripts to `book.toml`:
```toml
[output.html]
additional-js = ["theme/mermaid.min.js", "theme/mermaid-init.js"]
```

## NOTE

If you are using `mdbook serve`, you may want to add `src/generated/` to your `.gitignore` (in your book root) to prevent generated diagrams from invoking rebuilds.
