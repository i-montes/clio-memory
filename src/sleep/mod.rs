//! The sleep: once a night, while nobody is using the memory, it checks and copies the file,
//! quarantines secrets and likely attacks, folds repeats, merges near-duplicates when given a
//! model, and purges what was discarded long ago. See docs/motor.md.

pub mod signals;
