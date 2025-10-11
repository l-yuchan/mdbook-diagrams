![Crates.io Version](https://img.shields.io/crates/v/mdbook-diagrams)
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

Diagrams are generated at build time and saved at `src/generated/diagrams`. No need to include ~2.5MB js files!

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

## NOTE

If you are using `mdbook serve`, you would like to add `src/generated/` to your `.gitignore`(in your book root) to prevent generated diagrams invoking rebuild.
