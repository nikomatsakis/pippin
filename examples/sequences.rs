/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

extern crate byteorder;
extern crate rustc_serialize;
extern crate docopt;
extern crate pippin;
extern crate rand;
#[macro_use]
extern crate log;
extern crate env_logger;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::fs;
use std::cmp::min;
use std::u32;
use std::collections::hash_map::{HashMap, Entry};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use docopt::Docopt;
use rand::Rng;
use rand::distributions::{IndependentSample, Range, Normal, LogNormal};

use pippin::{ElementT, PartId, Partition, State, MutState, PartIO, PartState};
use pippin::discover::*;
use pippin::repo::*;
use pippin::merge::*;
use pippin::error::{Result, OtherError};


// —————  Sequence type  —————
type R = f64;
const R_SIZE: usize = 8;
#[derive(PartialEq, Debug)]
struct Sequence { v: Vec<R> }

impl ElementT for Sequence {
    fn write_buf(&self, writer: &mut Write) -> Result<()> {
        for x in &self.v {
            try!(writer.write_f64::<LittleEndian>(*x));
        }
        Ok(())
    }
    fn read_buf(buf: &[u8]) -> Result<Self> {
        if buf.len() % R_SIZE != 0 {
            return OtherError::err("invalid data length for a Sequence");
        }
        let mut r: &mut &[u8] = &mut &buf[..];
        let n = buf.len() / R_SIZE;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(try!(r.read_f64::<LittleEndian>()));
        }
        Ok(Sequence{ v: v })
    }
}

// —————  derived types for Repo  —————
#[derive(Clone)]
struct SeqClassifier {
    // For each class, the partition identifier and the min length of
    // sequences in the class. Ordered by min length, increasing.
    classes: Vec<(usize, PartId)>,
}

// Each classification has a PartId, a max PartId, a min length, a max length
// and a version number. The PartId is stored as the key.
#[derive(Clone)]
struct PartInfo {
    max_part_id: PartId,
    // Information version; number increased each time partition changes
    ver: u32,
    min_len: u32,
    max_len: u32,
}

struct SeqRepo<IO: RepoIO> {
    csf: SeqClassifier,
    io: IO,
    parts: HashMap<PartId, PartInfo>,
}
impl<IO: RepoIO> SeqRepo<IO> {
    pub fn new(r: IO) -> SeqRepo<IO> {
        SeqRepo {
            csf: SeqClassifier { classes: Vec::new() },
            io: r,
            parts: HashMap::new(),
        }
    }
    fn set_classifier(&mut self) {
        let mut classes = Vec::with_capacity(self.parts.len());
        for (part_id, part) in &self.parts {
            if part.max_len > part.min_len {
                classes.push((part.min_len as usize, part_id.clone()));
            }
        }
        // Note: there *could* be overlap of ranges. We can't do much if there
        // is and it won't cause failures later, so ignore this possibility.
        classes.sort_by(|a, b| a.0.cmp(&b.0));
        self.csf.classes = classes;
    }
}
impl ClassifierT for SeqClassifier {
    type Element = Sequence;
    fn classify(&self, elt: &Sequence) -> Option<PartId> {
        let len = elt.v.len();
        match self.classes.binary_search_by(|x| x.0.cmp(&len)) {
            Ok(i) => Some(self.classes[i].1), // len equals lower bound
            Err(i) => {
                if i == 0 {
                    None    // shouldn't happen, since we should have a class with lower bound 0
                } else {
                    // i is index such that self.classes[i-1].0 < len < self.classes[i].0
                    Some(self.classes[i-1].1)
                }
            }
        }
    }
    fn fallback(&self) -> ClassifyFallback {
        // classify() only returns None if something is broken; stop
        ClassifyFallback::Fail
    }
}
impl<IO: RepoIO> RepoT<SeqClassifier> for SeqRepo<IO> {
    fn repo_io(&mut self) -> &mut RepoIO {
        &mut self.io
    }
    fn clone_classifier(&self) -> SeqClassifier {
        self.csf.clone()
    }
    fn init_first(&mut self) -> Result<Box<PartIO>> {
        assert!(self.parts.is_empty());
        let p_id = PartId::from_num(1);
        self.parts.insert(p_id, PartInfo {
            max_part_id: PartId::from_num(PartId::max_num()),
            ver: 0,
            min_len: 0,
            max_len: u32::MAX,
        });
        self.set_classifier();
        try!(self.io.add_partition(p_id, ""));
        Ok(try!(self.io.make_partition_io(p_id)))
    }
    fn divide(&mut self, part: &PartState<Sequence>) ->
        Result<(Vec<PartId>, Vec<PartId>), RepoDivideError>
    {
        // 1: choose new lengths to use for partitioning
        // Algorithm: sample up to 999 lengths, find the median
        if part.num_avail() < 1 { return Err(RepoDivideError::NotSubdivisible); }
        let mut lens = Vec::with_capacity(min(999, part.num_avail()));
        for elt in part.elt_map() {
            let seq: &Sequence = elt.1;
            assert!(seq.v.len() <= u32::MAX as usize);
            lens.push(seq.v.len() as u32);
            if lens.len() >= 999 { break; }
        }
        lens.sort();
        let mid_point = lens.len() / 2;
        let median = lens[mid_point];
        // 1st new class uses existing lower-bound; 2nd uses median as its lower bound
        
        // 2: find new partition numbers
        let old_id = part.part_id();
        let old_num = old_id.into_num();
        let (max_num, min_len, max_len) = match self.parts.get(&old_id) {
            Some(part) => 
                (part.max_part_id.into_num(), part.min_len, part.max_len),
            None => {
                return Err(RepoDivideError::msg("missing info"));
            },
        };
        if max_num < old_num + 2 {
            // Not enough numbers
            // TODO: steal numbers from other partitions
            return Err(RepoDivideError::NotSubdivisible);
        }
        let num1 = old_num + 1;
        let num2 = num1 + (max_num - old_num) / 2;
        let (id1, id2) = (PartId::from_num(num1), PartId::from_num(num2));
        
        // 3: update and report
        let ver = self.parts.get(&id1).map_or(0, |pi| pi.ver + 1);
        self.parts.insert(id1, PartInfo {
            max_part_id: PartId::from_num(num2 - 1),
            ver: ver,
            min_len: min_len,
            max_len: median - 1,
        });
        let ver = self.parts.get(&id2).map_or(0, |pi| pi.ver + 1);
        self.parts.insert(id2, PartInfo {
            max_part_id: PartId::from_num(max_num),
            ver: ver,
            min_len: median,
            max_len: max_len,
        });
        if let Some(pi) = self.parts.get_mut(&old_id) {
            pi.max_part_id = old_id;
            pi.ver = pi.ver + 1;
            pi.max_len = pi.min_len;    // mark as no longer in use
        }
        self.set_classifier();
        //TODO: what happens with return value?
        Ok((vec![id1, id2], vec![]))
    }
    fn write_buf(&self, _num: PartId, w: &mut Write) -> Result<()> {
        // Classifier data (little endian):
        // "SeqCSF01" identifier
        // Number of PartInfos (u32)
        // PartInfos: two PartIds (u64 × 2), ver (u32), lengths (u32 × 2)
        try!(w.write(b"SeqCSF01"));
        assert!(self.parts.len() <= u32::MAX as usize);
        try!(w.write_u32::<LittleEndian>(self.parts.len() as u32));
        for (part_id, part) in &self.parts {
            try!(w.write_u64::<LittleEndian>((*part_id).into()));
            try!(w.write_u64::<LittleEndian>(part.max_part_id.into()));
            try!(w.write_u32::<LittleEndian>(part.ver));
            try!(w.write_u32::<LittleEndian>(part.min_len));
            try!(w.write_u32::<LittleEndian>(part.max_len));
        }
        Ok(())
    }
    fn read_buf(&mut self, _num: PartId, buf: &[u8]) -> Result<()> {
        // Format is as defined in `write_buf()`.
        
        //TODO: how should errors be handled? Clean up handling.
        if buf.len() < 12 {
            return OtherError::err("Insufficient data for classifier's read_buf()");
        }
        if buf[0..8] != *b"SeqCSF01" {
            return OtherError::err("Invalid format identifier for classifier's read_buf()");
        }
        let n_parts = try!((&mut &buf[8..12]).read_u32::<LittleEndian>()) as usize;
        if buf.len() != 12 + (8*2 + 4*3) * n_parts {
            return OtherError::err("Wrong data length for classifier's read_buf()");
        }
        let mut r = &mut &buf[12..];
        for _ in 0..n_parts {
            let part_id = try!(PartId::try_from(try!(r.read_u64::<LittleEndian>()))
                    .ok_or(OtherError::new("Invalid PartId for classifier's read_buf()")));
            let max_part_id = try!(PartId::try_from(try!(r.read_u64::<LittleEndian>()))
                    .ok_or(OtherError::new("Invalid PartId for classifier's read_buf()")));
            let ver = try!(r.read_u32::<LittleEndian>());
            let min_len = try!(r.read_u32::<LittleEndian>());
            let max_len = try!(r.read_u32::<LittleEndian>());
            
            match self.parts.entry(part_id) {
                Entry::Occupied(mut e) => {
                    if ver > e.get().ver {
                        // Replace all entries with more recent information
                        let v = e.get_mut();
                        v.max_part_id = max_part_id;
                        v.ver = ver;
                        v.min_len = min_len;
                        v.max_len = max_len;
                    }   // else: information is not newer; ignore
                },
                Entry::Vacant(e) => {
                    e.insert(PartInfo {
                        max_part_id: max_part_id,
                        ver: ver,
                        min_len: min_len,
                        max_len: max_len,
                    });
                },
            }
        }
        self.set_classifier();
        Ok(())
    }
}


// —————  Generators  —————
trait Generator {
    fn generate(&self, n: usize) -> Vec<R>;
}
struct Arithmetic { start: R, step: R }
struct Geometric { start: R, factor: R }
struct Fibonacci { x1: R, x2: R }
struct Power { e: R }

impl Generator for Arithmetic {
    fn generate(&self, n: usize) -> Vec<R> {
        let mut v = Vec::with_capacity(n);
        let mut x = self.start;
        while v.len() < n {
            v.push(x);
            x += self.step;
        }
        v
    }
}
impl Generator for Geometric {
    fn generate(&self, n: usize) -> Vec<R> {
        let mut v = Vec::with_capacity(n);
        let mut x = self.start;
        while v.len() < n {
            v.push(x);
            x *= self.factor;
        }
        v
    }
}
impl Generator for Fibonacci {
    fn generate(&self, n: usize) -> Vec<R> {
        let mut v = Vec::with_capacity(n);
        let (mut x1, mut x2) = (self.x1, self.x2);
        while v.len() < n {
            v.push(x1);
            let x = x1 + x2;
            x1 = x2;
            x2 = x;
        }
        v
    }
}
impl Generator for Power {
    fn generate(&self, n: usize) -> Vec<R> {
        let mut v = Vec::with_capacity(n);
        let mut i: R = 0.0;
        while v.len() < n {
            v.push(i.powf(self.e));
            i += 1.0;
        }
        v
    }
}


// —————  main  —————
const USAGE: &'static str = "
Generates or reads a database of sequences.

Usage:
  sequences [options]

Options:
  -h --help             Show this message.
  -d --directory DIR    Specify the directory to read/write the repository
  -p --partition BASE   Instead of a whole repository, just use the partition
                        with basename BASE (e.g. the basename of
                        'xyz-pn1-ss0.pip' is 'xyz-pn1').
  -c --create           Create a new repository
  -s --snapshot         Force creation of snapshot at end
  -g --generate NUM     Generate NUM new sequences and add to the repo.
  -R --repeat N         Repeat N times.

Note that you shouldn't try to create a partition with `-p`, then load that
partition alongside others as part of a repository; at least not without making
sure the repository name and partition number are correct.
";

#[derive(Debug, RustcDecodable)]
#[allow(non_snake_case)]
struct Args {
    flag_directory: Option<String>,
    flag_partition: Option<String>,
    flag_generate: Option<usize>,
    flag_create: bool,
    flag_snapshot: bool,
    flag_repeat: Option<usize>,
}

#[derive(PartialEq, Debug)]
enum Mode {
    Generate(usize),
    None
}

fn main() {
    env_logger::init().unwrap();
    
    let args: Args = Docopt::new(USAGE)
            .and_then(|d| d.decode())
            .unwrap_or_else(|e| e.exit());
    
    let dir = PathBuf::from(match args.flag_directory {
        Some(dir) => dir,
        None => {
            println!("Error: --directory option required (use --help for usage)");
            exit(1);
        },
    });
    if fs::create_dir_all(&dir).is_err() {
        println!("Unable to create/find directory {}", dir.display());
        exit(1);
    }
    
    let mode = match args.flag_generate {
        Some(num) => Mode::Generate(num),
        None => Mode::None,
    };
    
    let repetitions = args.flag_repeat.unwrap_or(1);
    
    let result = run(&dir, args.flag_partition, mode, args.flag_create,
            args.flag_snapshot, repetitions);
    if let Err(e) = result {
        println!("Error: {}", e);
        exit(1);
    }
}

fn run(dir: &Path, part_basename: Option<String>, mode: Mode, create: bool,
        snapshot: bool, repetitions: usize) -> Result<()>
{
    let solver1 = AncestorSolver2W::new();
    let solver2 = RenamingSolver2W::new();
    let merge_solver = TwoWaySolverChain::new(&solver1, &solver2);
    
    let mut rng = rand::thread_rng();
    let mut generate = |state: &mut MutState<_>| match mode {
        Mode::Generate(num) => {
            match Range::new(0, 4).ind_sample(&mut rng) {
                0 => {
                    let gen = Arithmetic{
                        start: LogNormal::new(0., 100.).ind_sample(&mut rng),
                        step: Normal::new(0., 10.).ind_sample(&mut rng),
                    };
                    generate(state, &mut rng, num, &gen);
                },
                1 => {
                    let gen = Geometric{
                        start: LogNormal::new(0., 100.).ind_sample(&mut rng),
                        factor: Normal::new(0., 2.).ind_sample(&mut rng),
                    };
                    generate(state, &mut rng, num, &gen);
                },
                2 => {
                    let gen = Fibonacci{
                        x1: Normal::new(1., 1.).ind_sample(&mut rng),
                        x2: Normal::new(1., 1.).ind_sample(&mut rng),
                    };
                    generate(state, &mut rng, num, &gen);
                },
                3 => {
                    let gen = Power{
                        e: LogNormal::new(0., 1.).ind_sample(&mut rng),
                    };
                    generate(state, &mut rng, num, &gen);
                },
                _ => { panic!("invalid sample"); }
            }
        },
        Mode::None => {},
    };
    
    if let Some(basename) = part_basename {
        let mut io = Box::new(try!(DiscoverPartFiles::from_dir_basename(dir, &basename, None)));
        if io.part_id() == None {
            // On creation or where discovery fails we need a number:
            io.set_part_id(PartId::from_num(1));
        }
            
        let mut part = if create {
            try!(Partition::<Sequence>::create(io, "sequences db"))
        } else {
            let mut part = try!(Partition::<Sequence>::open(io));
            try!(part.load(false));
            part
        };
        
        if part.merge_required() {
            try!(part.merge(&merge_solver));
        }
        
        for _ in 0..repetitions {
            let mut state = {
                let tip = try!(part.tip());
                println!("Found state {}; have {} elements", tip.statesum(), tip.num_avail());
                tip.clone_mut()
            };
            generate(&mut state);
            println!("Done modifying state");
            try!(part.push_state(state, None));
            try!(part.write(false));
        }
        
        if snapshot {
            try!(part.write_snapshot());
        }
    } else {
        let discover = try!(DiscoverRepoFiles::from_dir(dir));
        let rt = SeqRepo::new(discover);
        
        let mut repo = if create {
            try!(Repository::create(rt, "sequences db"))
        } else {
            let mut repo = try!(Repository::open(rt));
            try!(repo.load_all(false));
            repo
        };
        
        if repo.merge_required() {
            try!(repo.merge(&merge_solver));
        }
        
        for _ in 0..repetitions {
            let mut state = try!(repo.clone_state());
            println!("Found {} partitions; with {} elements", state.num_parts(), state.num_avail());
            generate(&mut state);
            println!("Done modifying state");
            try!(repo.merge_in(state));
            try!(repo.write_all(false));
        }
        
        if snapshot {
            try!(repo.write_snapshot_all());
        }
    }
    
    Ok(())
}

fn generate<R: Rng>(state: &mut MutState<Sequence>, rng: &mut R,
    num: usize, generator: &Generator)
{
    let len_range = LogNormal::new(1., 2.);
    let max_len = 1_000;
    let mut longest = 0;
    let mut total = 0;
    for _ in 0..num {
        let len = min(len_range.ind_sample(rng) as usize, max_len);
        if len > longest { longest = len; }
        total += len;
        let seq = Sequence{ v: generator.generate(len) };
        state.insert(seq).expect("insert element");
    }
    println!("Generated {} sequences; longest length {}, average {}", num, longest, (total as f64) / (num as f64));
}
