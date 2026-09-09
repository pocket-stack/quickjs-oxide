// Keep the binary-data oracle implementations in isolated modules while Cargo
// builds one integration target.

mod oracle_array_buffer;
mod oracle_data_view;
mod oracle_uint8array_codecs;
