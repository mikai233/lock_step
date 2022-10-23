use std::io::Result;

fn main() -> Result<()> {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();
    protobuf_codegen::Codegen::new()
        .protoc_path(&*protoc_path)
        // Use `protoc` parser, optional.
        // .protoc()
        // Use `protoc-bin-vendored` bundled protoc command, optional.
        // .protoc_path(&protoc_bin_vendored::protoc_bin_path().unwrap())
        // All inputs and imports from the inputs must reside in `includes` directories.
        .includes(&["src/proto"])
        // Inputs must reside in some of include paths.
        .input("src/proto/message.proto")
        .input("src/proto/cs.proto")
        .input("src/proto/sc.proto")
        // Specify output directory relative to Cargo output directory.
        .cargo_out_dir("protos")
        .run_from_script();
    Ok(())
}
