pub const DATA_LINE: &[u8] = b"data: Hello, world!\n";
pub const COMMENT_LINE: &[u8] = b": this is a comment\n";
pub const EVENT_LINE: &[u8] = b"event: update\n";
pub const ID_LINE: &[u8] = b"id: 42\n";
pub const EMPTY_LINE: &[u8] = b"\n";
pub const NO_VALUE_LINE: &[u8] = b"data\n";
pub const NO_SPACE_LINE: &[u8] = b"data:value\n";

// 1024 byte payload including some emoji
const GIANT_FAT_STUPID_DATA_LINE_STR: &str = "data: XKLlHDGrKnwBwvVvXx1\u{1F431}rE4FDydW9D6uQB4UcK6QSjMGDN6PdDna0iMD44ArdiK\u{1F431}IZjNxHwzbXz4Hkps1VVelGyeRjg\u{1F431}Wx8LZzl2wQa1gKW7JpgTJvkrF14khCAxftXkXmFZ1FkhOxDz9r\u{1F431}B6x8Xboeb0vKmETYTNOR\u{1F431}eKMI2ZbFQcLKcL\u{1F431}jFRzOm53Wwm5pKyaWeqwg0NXtZdcwJXQkFeKPHHPjbZGAijRq1YuayEs6ff6Fl82M2NNqwr\u{1F431}Y7pxb38lolpTpYjz\u{1F431}M4iGt89mKwevK7njRaQUXDlIJalc4SgSi\u{1F431}kTs79\u{1F431}6YjeCchiNUBq2Zw42LIjqbOSE4TAUGCdNdkV6PukVtvCIknAS4K2gqLIH\u{1F431}pV8fpG0fq2S3HHywv\u{1F431}AsPLZYEoQhtNVrCmVsKVpDlWgf\u{1F431}Zi1ZT3TuBnDoWM21ResbXy0FDZzQcmWFD\u{1F431}ZwfXJTGBU\u{1F431}\u{1F431}vRAd\u{1F431}ntTcNowfgZbcQtEnSiV9MR229uOePGD6AdD\u{1F431}Y9LQ\u{1F431}5tUVk9zykgeddis9eE8DxrzLYKVNlpHpSQ9yQhPTFifr0yDQ9H4WChQQwuqPpeNHGSqS7VSQkERne3cxNmILjR7n\u{1F431}E8RmEmgs9pZV6\u{1F431}OTCWF38pl1jVMG5BuVCKt9aV\u{1F431}GF4ukp3tv9\u{1F431}vnR8SZT6X\u{1F431}kFUB5MmSNpvMqj\u{1F431}uTayg1mhH\u{1F431}oYvZbJYqtnASAGeiTA3GC1C31FHgmNm42lNE2iCiml1GOT7npsgEyi4UrTPkG38aMyQ\u{1F431}bDc47bl4odbag7mpdPX\u{1F431}k0g\u{1F431}8ThsqoisJkH2nt\u{1F431}mWXVD1gU61IzM7u8OEz2LMHhzNbKjammpUDx8W6ABO8w15rKBkR\u{1F431}WI4sFLYUXCyZq\u{1F431}7dRqDK4q7RZg6GYpKKSFzvNklcZFZgUQcmnQLfSAxhW4kD\u{1F431}nVeHjtbuVoHEZ3icq\u{1F431}3l31\u{1F431}\n";
pub const BIG_OLE_DATA_LINE: &[u8] = GIANT_FAT_STUPID_DATA_LINE_STR.as_bytes();

pub fn generate_one_of_each(n: usize) -> Vec<u8> {
    let mut buf = Vec::<u8>::with_capacity(
        (DATA_LINE.len()
            + COMMENT_LINE.len()
            + EVENT_LINE.len()
            + ID_LINE.len()
            + EMPTY_LINE.len())
            * n,
    );

    for _ in 0..n {
        buf.extend_from_slice(DATA_LINE);
        buf.extend_from_slice(COMMENT_LINE);
        buf.extend_from_slice(EVENT_LINE);
        buf.extend_from_slice(ID_LINE);
        buf.extend_from_slice(EMPTY_LINE);
    }
    buf
}
