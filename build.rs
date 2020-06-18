use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

fn main() {
    let build_file_name = PathBuf::from("./assets/build.num");
    let (mut build_file, mut build_num) = if build_file_name.exists() {
        let mut build_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open(&build_file_name)
            .unwrap();
        let mut build_num = String::new();
        build_file.read_to_string(&mut build_num).unwrap();
        build_file.seek(SeekFrom::Start(0));
        (build_file, build_num.parse::<u32>().unwrap())
    } else {
        (
            OpenOptions::new()
                .read(false)
                .write(true)
                .create(true)
                .open(&build_file_name)
                .unwrap(),
            0,
        )
    };

    build_num += 1;
    build_file
        .write_all(build_num.to_string().as_bytes())
        .unwrap();

    println!("cargo:rerun-if-changed=build.rs");
}
