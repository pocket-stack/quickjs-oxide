// Keep the binary-data oracle implementations in isolated modules while Cargo
// builds one integration target.

#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

#[path = "oracle/binary_data/oracle_array_buffer.rs"]
mod oracle_array_buffer;
#[path = "oracle/binary_data/oracle_data_view.rs"]
mod oracle_data_view;
#[path = "oracle/binary_data/oracle_uint8array_codecs.rs"]
mod oracle_uint8array_codecs;
