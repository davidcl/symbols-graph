extern crate clap;
extern crate gimli;
extern crate object;
extern crate memmap;
extern crate string_interner;

use clap::{App, Arg};
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use object::Object;

struct Graph {
    name: String,

    nodes: HashMap<usize, NodeProperties>,
    edges: HashMap<(usize, usize), EdgeProperties>,

    clusters: Vec<Graph>,
    strings: string_interner::StringInterner<usize>
}

impl Graph {
    fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            nodes: HashMap::new(),
            edges: HashMap::new(),
            clusters: Vec::new(),
            strings: string_interner::StringInterner::new(),
        }
    }

    // parse a binary file using object
    fn parse_binary(&mut self, filename: &str) {
        let file = fs::File::open(filename);
        let file = match file {
            Ok(file) => file,
            Err(error) => panic!("Unable to open the file: {:?}", error)
        };

        let memory = unsafe { memmap::Mmap::map(&file) };
        let memory = match memory {
            Ok(memory) => memory,
            Err(error) => panic!("Unable to mmap the file: {:?}", error)
        };

        // parse the mapped file, borrowed by self.memory
        let object_file = object::File::parse(&*memory);
        let object_file = match object_file {
            Ok(object_file) => object_file,
            Err(error) => panic!("Unable to parse the file: {:?}", error)
        };

        // add the dynamic symbols to the graph
        for (_, sym) in object_file.dynamic_symbols() {
            self.insert(filename, sym);
        }

        // add the non-dynamic symbols to the graph (in case of plain object files)
        for (_, sym) in object_file.symbols() {
            self.insert(filename, sym);
        }
    }

    fn insert(&mut self, filename: &str, sym: object::Symbol) {
            if sym.name().unwrap_or("").is_empty() {
                return
            }

            let symbol_name = sym.name().unwrap();

            let filename = match self.mangle_as_valid_dot_name(filename) {
                Some(v) => v,
                None => return,
            };
            let symbol_name = match self.mangle_as_valid_dot_name(symbol_name) {
                Some(v) => v,
                None => return,
            };

            let filename = self.strings.get_or_intern(filename);
            let symbol_name = self.strings.get_or_intern(symbol_name);

            if sym.is_undefined() {
                self.edges.insert((filename, symbol_name), EdgeProperties {});
            } else {
                self.edges.insert((symbol_name, filename), EdgeProperties {});
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
        // __ prefixed symbols are compiler reserved
        if v.starts_with("__") {
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
}

impl Display for Graph {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "digraph {} {{", self.name)?;

        for (idx, v) in self.strings.iter() {
            if let Some(node_properties) = self.nodes.get(&idx) {
                writeln!(f, "    n{} [label=\"{}\";{}]", idx, v, node_properties)?;
            } else {
                writeln!(f, "    n{} [label=\"{}\"]", idx, v)?;
            }
        }
        for ((n1, n2), _) in &self.edges {
            writeln!(f, "    n{} -> n{}", n1, n2)?;
        }

        for c in &self.clusters {
            writeln!(f, "    {}", c)?;
        }
        writeln!(f, "}}")
    }
}

#[derive(Debug)]
struct NodeProperties {}

impl Display for NodeProperties {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "")
    }
}

#[derive(Debug)]
struct EdgeProperties {}

impl Display for EdgeProperties {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "")
    }
}

fn main() {
    let matches = App::new("Symbols graph")
        .version("0.1")
        .about("Parse shared objects and compute their internal and external dependencies.")
        .arg(
            Arg::with_name("output")
                .short("o")
                .help("Sets the output file")
                .required(false),
        )
        .arg(
            Arg::with_name("verbose")
                .short("v")
                .help("Sets the level of verbosity")
                .required(false),
        )
        .arg(
            Arg::with_name("file")
                .help("Sets the input file to use")
                .multiple(true)
                .required(true)
                .index(1),
        )
        .get_matches();

    // the file to write into
    let mut writer: Box<dyn Write> = match matches.value_of("output") {
        Some(output) => {
            let path = Path::new(output);
            Box::new(fs::File::create(&path).unwrap())
        }
        None => Box::new(io::stdout()),
    };

    // read inputs and write dot file directly
    let graph = if let Some(files) = matches.values_of("file") {
        let mut graph = Graph::new("");

        for f in files {
            if let Some(_) = matches.value_of("verbose") {
                println!("Parsing file {}", f);
            }

            graph.parse_binary(f);
        }

        graph
    } else {
        Graph::new("")
    };

    // write as dot format
    if let Some(_) = matches.value_of("verbose") {
        println!("Exporting graph");
    }
    write!(writer, "{}", graph).expect("Unable to write the graph");
}
