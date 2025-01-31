//! Check a raw message (part) for validity given a signature
//!
//! This could be useful for proxies that want to make sure they only forward valid messages. Since this does not
//! try to unmarshal anything it should be more efficient than doing a whole unmarshalling just to check for correctness.

use crate::signature;
use crate::wire::errors::UnmarshalError;
use crate::ByteOrder;

/// Either Ok(amount_of_bytes) or Err(position, ErrorCode)
pub type ValidationResult = Result<usize, (usize, UnmarshalError)>;

pub fn validate_marshalled(
    byteorder: ByteOrder,
    offset: usize,
    raw: &[u8],
    sig: &signature::Type,
) -> ValidationResult {
    match sig {
        signature::Type::Base(b) => validate_marshalled_base(byteorder, offset, raw, *b),
        signature::Type::Container(c) => validate_marshalled_container(byteorder, offset, raw, c),
    }
}

pub fn validate_marshalled_base(
    byteorder: ByteOrder,
    offset: usize,
    buf: &[u8],
    sig: signature::Base,
) -> ValidationResult {
    let padding = crate::wire::util::align_offset(sig.get_alignment(), buf, offset)
        .map_err(|err| (offset, err))?;

    match sig {
        signature::Base::Byte => {
            if buf[offset + padding..].is_empty() {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(1 + padding)
        }
        signature::Base::Uint16 => {
            if buf[offset + padding..].len() < 2 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(2 + padding)
        }
        signature::Base::Int16 => {
            if buf[offset + padding..].len() < 2 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(2 + padding)
        }
        signature::Base::Uint32 => {
            if buf[offset + padding..].len() < 4 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(4 + padding)
        }
        signature::Base::UnixFd => {
            if buf[offset + padding..].len() < 4 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(4 + padding)
        }
        signature::Base::Int32 => {
            if buf[offset + padding..].len() < 4 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(4 + padding)
        }
        signature::Base::Uint64 => {
            if buf[offset + padding..].len() < 8 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(8 + padding)
        }
        signature::Base::Int64 => {
            if buf[offset + padding..].len() < 8 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(8 + padding)
        }
        signature::Base::Double => {
            if buf[offset + padding..].len() < 8 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            Ok(8 + padding)
        }
        signature::Base::Boolean => {
            if buf[offset + padding..].len() < 4 {
                return Err((offset + padding, UnmarshalError::NotEnoughBytes));
            }
            let offset = offset + padding;
            let slice = &buf[offset..offset + 4];
            let (_, val) =
                crate::wire::util::parse_u32(slice, byteorder).map_err(|err| (offset, err))?;
            match val {
                0 => Ok(4 + padding),
                1 => Ok(4 + padding),
                _ => Err((offset, UnmarshalError::InvalidBoolean)),
            }
        }
        signature::Base::String => {
            let offset = offset + padding;
            let (bytes, _string) = crate::wire::util::unmarshal_str(byteorder, &buf[offset..])
                .map_err(|err| (offset, err))?;
            Ok(bytes + padding)
        }
        signature::Base::ObjectPath => {
            let offset = offset + padding;
            let (bytes, string) = crate::wire::util::unmarshal_str(byteorder, &buf[offset..])
                .map_err(|err| (offset, err))?;
            crate::params::validate_object_path(string).map_err(|e| (offset, e.into()))?;
            Ok(bytes + padding)
        }
        signature::Base::Signature => {
            let (bytes, string) = crate::wire::util::unmarshal_signature(&buf[offset..])
                .map_err(|err| (offset + padding, err))?;
            crate::params::validate_signature(string).map_err(|e| (offset, e.into()))?;
            Ok(bytes + padding)
        }
    }
}

use crate::wire::util;

pub fn validate_marshalled_container(
    byteorder: ByteOrder,
    offset: usize,
    buf: &[u8],
    sig: &signature::Container,
) -> ValidationResult {
    match sig {
        signature::Container::Array(elem_sig) => {
            let padding = util::align_offset(4, buf, offset).map_err(|err| (offset, err))?;
            let offset = offset + padding;
            let (_, bytes_in_array) =
                util::parse_u32(&buf[offset..], byteorder).map_err(|err| (offset, err))?;
            let offset = offset + 4;

            if buf[offset..].len() < bytes_in_array as usize {
                return Err((offset, UnmarshalError::NotEnoughBytesForCollection));
            }

            let first_elem_padding = util::align_offset(elem_sig.get_alignment(), buf, offset)
                .map_err(|err| (offset, err))?;
            let offset = offset + first_elem_padding;

            if buf[offset..].len() < bytes_in_array as usize {
                return Err((offset, UnmarshalError::NotEnoughBytesForCollection));
            }

            if elem_sig.bytes_always_valid() {
                // bytes_always_valid() only returns true for types whose
                // length is equal to their alignment
                if bytes_in_array as usize % elem_sig.get_alignment() != 0 {
                    // there is not a whole number of elements in the array.
                    return Err((offset, UnmarshalError::NotEnoughBytes));
                }
            } else {
                let mut bytes_used_counter = 0;
                let array_end = offset + bytes_in_array as usize;
                while bytes_used_counter < bytes_in_array as usize {
                    let bytes_used = validate_marshalled(
                        byteorder,
                        offset + bytes_used_counter,
                        &buf[..array_end],
                        elem_sig,
                    )?;
                    bytes_used_counter += bytes_used;
                }
            }
            let total_bytes_used = padding + 4 + first_elem_padding + bytes_in_array as usize;
            Ok(total_bytes_used)
        }
        signature::Container::Dict(key_sig, val_sig) => {
            let padding = util::align_offset(4, buf, offset).map_err(|err| (offset, err))?;
            let offset = offset + padding;
            let (_, bytes_in_dict) =
                util::parse_u32(&buf[offset..], byteorder).map_err(|err| (offset, err))?;
            let offset = offset + 4;

            if buf[offset..].len() < bytes_in_dict as usize {
                return Err((offset, UnmarshalError::NotEnoughBytesForCollection));
            }

            let before_elements_padding =
                util::align_offset(8, buf, offset).map_err(|err| (offset, err))?;
            let offset = offset + before_elements_padding;

            if buf[offset..].len() < bytes_in_dict as usize {
                return Err((offset, UnmarshalError::NotEnoughBytesForCollection));
            }

            // don't let the contents of the dict see anything beyond the dicts claimed end.
            let buf_for_dict = &buf[..offset + bytes_in_dict as usize];

            let mut bytes_used_counter = 0;
            while bytes_used_counter < bytes_in_dict as usize {
                let element_padding =
                    util::align_offset(8, buf_for_dict, offset + bytes_used_counter)
                        .map_err(|err| (offset + bytes_used_counter, err))?;
                bytes_used_counter += element_padding;
                let key_bytes = validate_marshalled_base(
                    byteorder,
                    offset + bytes_used_counter,
                    buf_for_dict,
                    *key_sig,
                )?;
                bytes_used_counter += key_bytes;
                let val_bytes = validate_marshalled(
                    byteorder,
                    offset + bytes_used_counter,
                    buf_for_dict,
                    val_sig,
                )?;
                bytes_used_counter += val_bytes;
            }
            Ok(padding + before_elements_padding + 4 + bytes_used_counter)
        }
        signature::Container::Struct(sigs) => {
            let padding = util::align_offset(8, buf, offset).map_err(|err| (offset, err))?;
            let offset = offset + padding;

            let mut bytes_used_counter = 0;
            for field_sig in sigs.as_ref() {
                let bytes_used =
                    validate_marshalled(byteorder, offset + bytes_used_counter, buf, field_sig)?;
                bytes_used_counter += bytes_used;
            }
            Ok(padding + bytes_used_counter)
        }
        signature::Container::Variant => {
            let (sig_bytes_used, sig_str) =
                util::unmarshal_signature(&buf[offset..]).map_err(|err| (offset, err))?;
            let mut sig =
                signature::Type::parse_description(sig_str).map_err(|e| (offset, e.into()))?;
            if sig.len() != 1 {
                // There must be exactly one type in the signature!
                return Err((offset, UnmarshalError::WrongSignature));
            }
            let sig = sig.remove(0);
            let offset = offset + sig_bytes_used;

            let param_bytes_used = validate_marshalled(byteorder, offset, buf, &sig)?;
            Ok(sig_bytes_used + param_bytes_used)
        }
    }
}

#[test]
fn test_raw_validation() {
    // make sure it catches errors
    let too_short_string = vec![13, 0, 0, 0, b'a', b'b', b'c'];
    assert_eq!(
        validate_marshalled(
            ByteOrder::LittleEndian,
            0,
            &too_short_string,
            &signature::Type::Base(signature::Base::String),
        )
        .err()
        .unwrap(),
        (0usize, UnmarshalError::NotEnoughBytes)
    );

    // 8u8 ++ padding ++ 14u32 with a 1 in the padding
    let data_in_padding = vec![8, 0, 1, 0, 14, 0, 0, 0];
    assert_eq!(
        validate_marshalled(
            ByteOrder::LittleEndian,
            0,
            &data_in_padding,
            &signature::Type::parse_description("(yu)").unwrap()[0],
        )
        .err()
        .unwrap(),
        (1usize, UnmarshalError::PaddingContainedData)
    );

    // padding of empty array
    let empty_array = vec![0, 0, 0, 0];
    assert_eq!(
        validate_marshalled(
            ByteOrder::LittleEndian,
            0,
            &empty_array,
            &signature::Type::parse_description("a(y)").unwrap()[0],
        )
        .err()
        .unwrap(),
        (4usize, UnmarshalError::NotEnoughBytes)
    );

    // padding of empty array
    let empty_array = vec![0, 0, 0, 0, 100, 0, 0, 0];
    assert_eq!(
        validate_marshalled(
            ByteOrder::LittleEndian,
            0,
            &empty_array,
            &signature::Type::parse_description("a(y)u").unwrap()[0],
        )
        .err()
        .unwrap(),
        (4usize, UnmarshalError::PaddingContainedData)
    );

    // just to make sure stuff that should pass does pass this
    let mut map = std::collections::HashMap::new();
    map.insert("A", (10u8, 100i64));
    map.insert("B", (80u8, 180i64));
    use crate::wire::marshal::MarshalContext;
    use crate::Marshal;

    let mut fds = Vec::new();
    let mut valid_buf = Vec::new();
    let mut ctx = MarshalContext {
        buf: &mut valid_buf,
        fds: &mut fds,
        byteorder: ByteOrder::LittleEndian,
    };
    let ctx = &mut ctx;

    (vec![(255u8, 4u32, true, u64::MAX)].as_slice(), map)
        .marshal(ctx)
        .unwrap();

    validate_marshalled(
        ByteOrder::LittleEndian,
        0,
        &valid_buf,
        &signature::Type::parse_description("(a(yubt)a{s(yx)})").unwrap()[0],
    )
    .unwrap();
    fds.clear();
    valid_buf.clear();

    let mut ctx = MarshalContext {
        buf: &mut valid_buf,
        fds: &mut fds,
        byteorder: ByteOrder::LittleEndian,
    };

    // make sure there is a padding between bytecount and start of the first element in the array
    let array: &[(&str,)] = &[("Hello, ",), ("World!",)];
    array.marshal(&mut ctx).unwrap();
    validate_marshalled(
        ByteOrder::LittleEndian,
        0,
        &valid_buf,
        &signature::Type::parse_description("a(s)").unwrap()[0],
    )
    .unwrap();
}
#[test]
fn test_array_element_overflow() {
    let mut buf = vec![10, 0, 0, 0, 10, 0, 0, 0];
    buf.resize(18, 0x61);
    buf.push(0);
    let typ = &signature::Type::parse_description("as").unwrap();
    validate_marshalled(ByteOrder::LittleEndian, 0, &buf, &typ[0]).unwrap_err();
}
