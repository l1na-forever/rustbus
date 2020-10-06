//! Marshal container params into raw bytes

use crate::params;
use crate::params::message;
use crate::signature;
use crate::wire::marshal::base::*;
use crate::wire::marshal::MarshalContext;
use crate::wire::util::*;

pub fn marshal_param(p: &params::Param, ctx: &mut MarshalContext) -> message::Result<()> {
    match p {
        params::Param::Base(b) => marshal_base_param(&b, ctx),
        params::Param::Container(c) => marshal_container_param(&c, ctx),
    }
}

fn marshal_array(
    array: &[params::Param],
    sig: &signature::Type,
    ctx: &mut MarshalContext,
) -> message::Result<()> {
    pad_to_align(4, ctx.buf);
    let len_pos = ctx.buf.len();
    ctx.buf.push(0);
    ctx.buf.push(0);
    ctx.buf.push(0);
    ctx.buf.push(0);

    // we need to pad here because the padding between length and first element does not count
    // into the length
    pad_to_align(sig.get_alignment(), ctx.buf);
    let content_pos = ctx.buf.len();
    for p in array {
        marshal_param(&p, ctx)?;
    }
    let len = ctx.buf.len() - content_pos;
    insert_u32(
        ctx.byteorder,
        len as u32,
        &mut ctx.buf[len_pos..len_pos + 4],
    );
    Ok(())
}

fn marshal_struct(params: &[params::Param], ctx: &mut MarshalContext) -> message::Result<()> {
    pad_to_align(8, ctx.buf);
    for p in params {
        marshal_param(&p, ctx)?;
    }
    Ok(())
}

fn marshal_variant(var: &params::Variant, ctx: &mut MarshalContext) -> message::Result<()> {
    let mut sig_str = String::new();
    var.sig.to_str(&mut sig_str);
    marshal_signature(&sig_str, ctx.buf)?;
    marshal_param(&var.value, ctx)?;
    Ok(())
}

fn marshal_dict(dict: &params::DictMap, ctx: &mut MarshalContext) -> message::Result<()> {
    pad_to_align(4, ctx.buf);
    let len_pos = ctx.buf.len();
    ctx.buf.push(0);
    ctx.buf.push(0);
    ctx.buf.push(0);
    ctx.buf.push(0);
    pad_to_align(8, ctx.buf);

    let content_pos = ctx.buf.len();
    for (key, value) in dict {
        pad_to_align(8, ctx.buf);
        marshal_base_param(&key, ctx)?;
        marshal_param(&value, ctx)?;
    }
    let len = ctx.buf.len() - content_pos;
    insert_u32(
        ctx.byteorder,
        len as u32,
        &mut ctx.buf[len_pos..len_pos + 4],
    );
    Ok(())
}

pub fn marshal_container_param(
    p: &params::Container,
    ctx: &mut MarshalContext,
) -> message::Result<()> {
    match p {
        params::Container::Array(params) => {
            params::validate_array(&params.values, &params.element_sig)?;
            marshal_array(&params.values, &params.element_sig, ctx)?;
        }
        params::Container::ArrayRef(params) => {
            params::validate_array(&params.values, &params.element_sig)?;
            marshal_array(&params.values, &params.element_sig, ctx)?;
        }
        params::Container::Struct(params) => {
            marshal_struct(params, ctx)?;
        }
        params::Container::StructRef(params) => {
            marshal_struct(params, ctx)?;
        }
        params::Container::Dict(params) => {
            params::validate_dict(&params.map, params.key_sig, &params.value_sig)?;
            marshal_dict(&params.map, ctx)?;
        }
        params::Container::DictRef(params) => {
            params::validate_dict(&params.map, params.key_sig, &params.value_sig)?;
            marshal_dict(params.map, ctx)?;
        }
        params::Container::Variant(variant) => {
            marshal_variant(variant, ctx)?;
        }
    }
    Ok(())
}
