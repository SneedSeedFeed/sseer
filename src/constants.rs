use bytes_utils::Str;

pub(crate) const LF: u8 = b'\n';
pub(crate) const CR: u8 = b'\r';

const BOM_CHAR: char = '\u{FEFF}';
const BOM_LEN: usize = BOM_CHAR.len_utf8();
// bom           = %xFEFF ; U+FEFF BYTE ORDER MARK
pub(crate) const BOM: &[u8; BOM_LEN] = &{
    let mut buf = [0u8; BOM_LEN];
    BOM_CHAR.encode_utf8(&mut buf);
    buf
};

pub(crate) const EMPTY_STR: Str = Str::from_static("");
pub(crate) const MESSAGE_STR: Str = Str::from_static("message");
