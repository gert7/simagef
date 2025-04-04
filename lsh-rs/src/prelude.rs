//! Re-export of the public api of lsh-rs.
pub use crate::{
    error::{Error, Result},
    hash::{MinHash, SignRandomProjections, VecHash, L2, MIPS},
    lsh::lsh::LSH,
    multi_probe::{QueryDirectedProbe, StepWiseProbe},
    table::{general::HashTables, mem::MemoryTable},
};

pub type LshMem<H, N = f32, K = i8> = LSH<H, N, MemoryTable<N, K>, K>;

macro_rules! concrete_lsh_structs {
    ($mod_name:ident, $K:ty) => {
        pub mod $mod_name {
            use super::*;
            pub type LshMem<H, N = f32> = LSH<H, N, MemoryTable<N, $K>, $K>;
        }
    };
}
concrete_lsh_structs!(hi8, i8);
concrete_lsh_structs!(hi16, i16);
concrete_lsh_structs!(hi32, i32);
concrete_lsh_structs!(hi64, i64);
