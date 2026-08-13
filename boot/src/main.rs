//! 内核二进制入口:真实入口在汇编 boot.S(_start)。
#![no_std]
#![no_main]

pub use mcs_kernel::*;