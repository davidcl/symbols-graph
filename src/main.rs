#![allow(unknown_lints)]
#![warn(clippy::all)]

extern crate clap;
extern crate object;
extern crate memmap;
extern crate string_interner;

use clap::{Command, Arg, ArgAction};
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::str;
use object::{Object, ObjectSymbol};

struct Graph {
    name: String,

    nodes: HashMap<usize, NodeProperties>,
    edges: HashMap<(usize, usize), EdgeProperties>,

    clusters: Vec<SubGraph>,
    strings: string_interner::StringInterner<usize>,
    
    // temporary map undefined symbol ->  lib
    undefined: HashMap<usize, Vec<usize>>,
    // temporary map defined symbol -> lib 
    defined: HashMap<usize, Vec<usize>>,
}

impl Graph {
    fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            nodes: HashMap::new(),
            edges: HashMap::new(),

            clusters: Vec::new(),
            strings: string_interner::StringInterner::new(),
            
            undefined: HashMap::new(),
            defined: HashMap::new(),
        }
    }

    // parse a binary file using object
    fn parse_binary(&mut self, filename: &str) {
        let file = fs::File::open(filename);
        let file = match file {
            Ok(file) => file,
            Err(error) => panic!("Unable to open {} : {:?}", filename, error)
        };

        let memory = unsafe { memmap::Mmap::map(&file) };
        let memory = match memory {
            Ok(memory) => memory,
            Err(error) => panic!("Unable to mmap {} : {:?}", filename, error)
        };

        // parse the mapped file, borrowed by memory
        let object_file = object::File::parse(&*memory);
        if let Err(error) = object_file {
            eprintln!("Unable to parse {} : {:?}", filename, error);
            return
        }
        let object_file = object_file.unwrap();

        let filename = match self.mangle_as_valid_dot_name(filename) {
            Some(v) => v,
            None => return,
        };

        let filename = self.strings.get_or_intern(filename);
        let mut properties = NodeProperties { symbols: vec![] };
        
        // add the exported symbols to the graph
        if let Ok(symbols) = object_file.exports() {
            for sym in symbols {
                self.insert_exported(&mut properties, filename, sym.name());
            }
        }

        // add the imported symbols to the graph (in case of plain object files)
        if let Ok(symbols) = object_file.imports() {
            for sym in symbols {
                self.insert_imported(&mut properties, filename, sym.name());
            }
        }

        self.nodes.insert(filename, properties);
    }

    fn insert_exported(&mut self, properties: &mut NodeProperties, filename: usize, exported_symbol: &[u8]) {
        let symbol_name = str::from_utf8(exported_symbol).unwrap();

        let symbol_name = match self.mangle_as_valid_dot_name(symbol_name) {
            Some(v) => v,
            None => return,
        };

        let symbol_name = self.strings.get_or_intern(symbol_name);

        // render in the label
        properties.symbols.push(symbol_name);

        // store for later resolution
        if let Some(libs) = self.defined.get_mut(&filename) {
            libs.push(filename);
        } else {
            self.defined.insert(symbol_name, vec![filename]);
        }

        // cleanup undefined if needed
        if let Some((_, libs)) = self.undefined.remove_entry(&symbol_name) {
            for lib in libs.iter() {
                let edge = (*lib, filename);
                if let Some(properties) = self.edges.get_mut(&edge) {
                    properties.symbols.push(symbol_name);
                } else {
                    self.edges.insert(edge, EdgeProperties { symbols: vec![symbol_name]});
                }
            }
        }
    }

    fn insert_imported(&mut self, properties: &mut NodeProperties, filename: usize, imported_symbol: &[u8]) {
        let symbol_name = str::from_utf8(imported_symbol).unwrap();

        let symbol_name = match self.mangle_as_valid_dot_name(symbol_name) {
            Some(v) => v,
            None => return,
        };

        let symbol_name = self.strings.get_or_intern(symbol_name);

        // lookup on existing libs
        if let Some(libs) = self.defined.get(&symbol_name) {
            // resolve to previously decoded libs 
            for lib in libs.iter() {
                let edge = (filename, *lib);
                if let Some(properties) = self.edges.get_mut(&edge) {
                    properties.symbols.push(symbol_name);
                } else {
                    self.edges.insert(edge, EdgeProperties { symbols: vec![symbol_name]});
                }
            }
        } else {
            // will be resolved later, store it
            if let Some(libs) = self.undefined.get_mut(&symbol_name) {
                libs.push(filename);
            } else {
                self.undefined.insert(symbol_name, vec![filename]);
            }
        }
    }

    fn mangle_as_valid_dot_name(&self, v: &str) -> Option<String> {
        // blacklisted symbols
        let v = match &v[0..] {
            "_GLOBAL_OFFSET_TABLE_" => return None,
            "" => return None,
            _ => v,
        };

        // .LC0 and .LC1 are used for constants
        if v.starts_with(".LC") {
            return None;
        }
        // _ prefixed symbols are compiler reserved
        if v.starts_with('_') {
            return None;
        }

        // escape file names: return basename
        let dot = if v.ends_with(".o") {
            v.len() - 2
        } else {
            v.len()
        };
        let slash = match v.rfind('/') {
            Some(index) => index+1,
            None => 0,
        };

        // filter invalid dot symbols
        Some(v[slash..dot].chars()
            // dot use dash as a edge symbol, translate it
            .map(|c: char| if c == '-' { '_' } else { c })
            // dot use dot as a edge symbol, translate it
            .map(|c: char| if c == '.' { '_' } else { c })
            .collect::<String>())
    }

    // remove all labels information from edges
    fn merge(&mut self) {
        for e in self.edges.values_mut() {
            e.symbols.clear();
        }
    }
}

impl Display for Graph {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "digraph {} {{", self.name)?;

        for c in &self.clusters {
            if let Some(label) = self.strings.resolve(c.name) {
                writeln!(f, "    subgraph {} {{", label)?;
            } else {
                writeln!(f, "    subgraph {{")?;
            }

            for (idx, _) in c.nodes.iter() {
                if let Some(label) = self.strings.resolve(*idx) {
                    writeln!(f, "        n{} [label=\"{}\"]", idx, label)?;
                } else {
                    writeln!(f, "        n{}", idx)?;
                }
            }

            writeln!(f, "    }}")?;
        }

        for (idx, _) in self.nodes.iter() {
            if let Some(label) = self.strings.resolve(*idx) {
                writeln!(f, "    n{} [label=\"{}\"]", idx, label)?;
            }
        }

        for ((n1, n2), p) in &self.edges {
            if p.symbols.len() == 0 {
                writeln!(f, "    n{} -> n{}", n1, n2)?;
            } else {
                for symbol in p.symbols.iter() {
                    if let Some(label) = self.strings.resolve(*symbol) {
                        writeln!(f, "    n{} -> n{} [label=\"{}\"]", n1, n2, label)?;
                    }
                }
            }
        }

        writeln!(f, "}}")
    }
}

#[derive(Debug)]
struct NodeProperties {
    symbols: Vec<usize>,
}

#[derive(Debug)]
struct EdgeProperties {
    symbols: Vec<usize>,
}

#[derive(Debug)]
struct SubGraph {
    name: usize,
    nodes: HashMap<usize, NodeProperties>
}

impl SubGraph {
    fn new(name: usize) -> Self {
        Self {
            name: name,
            nodes: HashMap::new()
        }
    }
    
    fn insert(&mut self, symbol_name: usize) {
        self.nodes.insert(symbol_name, NodeProperties { symbols: vec![] });
    }
}

fn main() {
    let matches = Command::new("Symbols graph")
        .version("0.1")
        .about("Parse shared objects and compute their internal and external dependencies.")
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .action(clap::ArgAction::SetTrue)
                .help("Sets the level of verbosity")
                .required(false),
        )
        .arg(
            Arg::new("merge")
                .short('m')
                .long("merge")
                .action(clap::ArgAction::SetTrue)
                .help("Generate only one edge between libraries")
                .required(false),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .num_args(1)
                .help("Sets the output file")
                .action(ArgAction::Set)
                .required(false),
        )
        .arg(
            Arg::new("file")
                .help("Sets the input file to use")
                .action(ArgAction::Append)
                .required(true),
        )
        .get_matches();

    // the file to write into
    let mut writer: Box<dyn Write> = match matches.get_one::<String>("output") {
        Some(output) => {
            let path = Path::new(output);
            Box::new(fs::File::create(&path).unwrap())
        }
        None => Box::new(io::stdout()),
    };

    // read inputs and write dot file directly
    let graph = if let Some(files) = matches.get_many::<String>("file") {
        let mut graph = Graph::new("");

        for f in files {
            if matches.get_flag("verbose") {
                println!("Parsing file {}", f);
            }

            graph.parse_binary(f);
        }

        if matches.get_flag("merge") {
            if matches.get_flag("verbose") {
                println!("merging");
            }
            graph.merge();
        }

        graph
    } else {
        Graph::new("")
    };

    // write as dot format
    if matches.get_flag("verbose") {
        println!("Exporting graph");
    }
    write!(writer, "{}", graph).expect("Unable to write the graph");
}
