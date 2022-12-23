use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use once_cell::sync::OnceCell;

use crate::ToUIntIP;

const HEADER_INFO_LENGTH: usize = 256;
const VECTOR_INDEX_COLS: usize = 256;
const VECTOR_INDEX_SIZE: usize = 8;
const SEGMENT_INDEX_SIZE: usize = 14;
const VECTOR_INDEX_LENGTH: usize = 512 * 1024;

const XDB_FILEPATH_ENV: &str = "XDB_FILEPATH";
const CACHE_POLICY_ENV: &str = "CACHE_POLICY";

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CachePolicy {
    Never=1,
    VecIndex,
    Full,
}

/// check https://mp.weixin.qq.com/s/ndjzu0BgaeBmDOCw5aqHUg for details
pub fn search_by_ip<T>(ip: T) -> Result<String, Box<dyn Error>>
    where
        T: ToUIntIP + Display,
{
    let ip = ip.to_u32_ip()?;
    let (start_ptr, end_ptr) = get_start_end_ptr(ip);
    let mut left: usize = 0;
    let mut right: usize = (end_ptr - start_ptr) / SEGMENT_INDEX_SIZE;

    while left <= right {
        let mid = (left + right) >> 1;
        let offset = start_ptr + mid * SEGMENT_INDEX_SIZE;
        let buffer_ip_value = &get_full_cache()[offset..offset+SEGMENT_INDEX_SIZE];
        let start_ip = get_block_by_size(buffer_ip_value, 0, 4);
        if ip < (start_ip as u32) {
            right = mid - 1;
        } else if ip > (get_block_by_size(buffer_ip_value, 4, 4) as u32) {
            left = mid + 1;
        } else {
            let data_length = get_block_by_size(buffer_ip_value, 8, 2);
            let data_offset = get_block_by_size(buffer_ip_value, 10, 4);
            let result = String::from_utf8(get_full_cache()[data_offset..(data_offset + data_length)].to_vec());
            return Ok(result?);
        }
    }
    Err("not matched".into())
}

pub fn get_start_end_ptr(ip: u32) -> (usize, usize) {
    let il0 = ((ip >> 24) & 0xFF) as usize;
    let il1 = ((ip >> 16) & 0xFF) as usize;
    let idx = VECTOR_INDEX_SIZE * (il0 * VECTOR_INDEX_COLS + il1);
    let start_point = idx;
    let vector_cache = get_vector_index_cache();
    let start_ptr = get_block_by_size( vector_cache, start_point, 4);
    let end_ptr = get_block_by_size(vector_cache, start_point + 4, 4);
    (start_ptr, end_ptr)
}

/// it will check ../data/ip2region.xdb, ../../data/ip2region.xdb, ../../../data/ip2region.xdb
fn default_detect_xdb_file() -> Result<String, Box<dyn Error>> {
    let prefix = "../".to_owned();
    for recurse in 1..4 {
        let filepath = prefix.repeat(recurse) + "data/ip2region.xdb";
        if Path::new(filepath.as_str()).exists() {
            return Ok(filepath);
        }
    }
    Err("default filepath not find the xdb file, so you must set xdb_filepath".into())
}

#[inline]
pub fn get_block_by_size(bytes: &[u8], offset: usize, length: usize) -> usize
{
    let mut result: usize = 0;
    for (index, value) in bytes[offset..offset + length].iter().enumerate() {
        result |= usize::from(value.clone()) << (index * 8);
    }
    result
}

fn set_log_level() {
    let rust_log_key = "RUST_LOG";
    std::env::var(rust_log_key).unwrap_or_else(|_| {
        std::env::set_var(rust_log_key, "INFO");
        std::env::var(rust_log_key).unwrap()
    });
}

pub fn searcher_init(xdb_filepath: Option<String>, cache_policy: Option<CachePolicy>) {
    set_log_level();
    let xdb_filepath = xdb_filepath.unwrap_or_else(|| {
        default_detect_xdb_file().unwrap()
    });
    std::env::set_var(XDB_FILEPATH_ENV, xdb_filepath.as_str());
    if let Some(policy) = cache_policy {
        std::env::set_var(CACHE_POLICY_ENV, policy);
        return;
    }
    std::env::set_var(CACHE_POLICY_ENV, CachePolicy::Full);

}

fn get_vector_index_cache() -> &'static [u8] {
    let full_cache: &'static Vec<u8> = get_full_cache();
    &full_cache[HEADER_INFO_LENGTH..(HEADER_INFO_LENGTH + VECTOR_INDEX_LENGTH)]
}

fn load_file() -> Vec<u8>{
    let xdb_filepath = std::env::var("XDB_FILEPATH").unwrap();
    tracing::debug!("load xdb searcher file at {} ", xdb_filepath);
    let mut f = File::open(xdb_filepath).expect("file open error");
    let mut buffer = Vec::new();
    f.read_to_end(&mut buffer).expect("load file error");
    buffer
}

fn get_full_cache() -> &'static Vec<u8> {
    let cache_policy = std::env::var(CACHE_POLICY_ENV).unwrap();
    if cache_policy == CachePolicy::Full {
        static CACHE: OnceCell<Vec<u8>> = OnceCell::new();
        return CACHE.get_or_init(|| load_file())
    }
    &load_file()
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::str::FromStr;
    use std::thread;
    use std::fs::File;
    use std::io::Read;

    use super::*;

    ///test all types find correct
    #[test]
    fn test_multi_type_ip() {
        searcher_init(None, None);

        search_by_ip("2.0.0.0").unwrap();
        search_by_ip("32").unwrap();
        search_by_ip(4294408949).unwrap();
        search_by_ip(Ipv4Addr::from_str("1.1.1.1").unwrap()).unwrap();
    }

    #[test]
    fn test_match_all_ip_correct() {
        searcher_init(None, None);
        let mut file = File::open("../../../data/ip.test.txt").unwrap();
        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();
        for line in contents.split("\n") {
            if !line.contains("|") {
                continue;
            }
            let ip_test_line = line.splitn(3, "|").collect::<Vec<&str>>();
            let start_ip = Ipv4Addr::from_str(ip_test_line[0]).unwrap();
            let end_ip = Ipv4Addr::from_str(ip_test_line[1]).unwrap();
            for value in u32::from(start_ip)..u32::from(end_ip) + 1 {
                let result = search_by_ip(value).unwrap();
                assert_eq!(result.as_str(), ip_test_line[2])
            }
        }
    }

    #[test]
    fn test_multi_thread_only_load_xdb_once() {
        searcher_init(None, None);
        let handle = thread::spawn(|| {
            let result =search_by_ip("2.2.2.2").unwrap();
            println!("ip search in spawn: {result}");
        });
        let r = search_by_ip("1.1.1.1").unwrap();
        println!("ip search in main thread: {r}");
        handle.join().unwrap();
    }
}
