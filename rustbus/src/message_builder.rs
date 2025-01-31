//! Build new messages that you want to send over a connection
use crate::params::message;
use crate::signature::SignatureIter;
use crate::wire::errors::MarshalError;
use crate::wire::errors::UnmarshalError;
use crate::wire::marshal::traits::{Marshal, SignatureBuffer};
use crate::wire::marshal::MarshalContext;
use crate::wire::unmarshal::UnmarshalContext;
use crate::wire::validate_raw;
use crate::wire::UnixFd;
use crate::ByteOrder;

/// Types a message might have
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MessageType {
    Signal,
    Error,
    Call,
    Reply,
    Invalid,
}

/// Flags that can be set in the message header
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum HeaderFlags {
    NoReplyExpected,
    NoAutoStart,
    AllowInteractiveAuthorization,
}

impl HeaderFlags {
    pub fn into_raw(self) -> u8 {
        match self {
            HeaderFlags::NoReplyExpected => 1,
            HeaderFlags::NoAutoStart => 2,
            HeaderFlags::AllowInteractiveAuthorization => 4,
        }
    }

    pub fn is_set(self, flags: u8) -> bool {
        flags & self.into_raw() == 1
    }

    pub fn set(self, flags: &mut u8) {
        *flags |= self.into_raw()
    }

    pub fn unset(self, flags: &mut u8) {
        *flags &= 0xFF - self.into_raw()
    }
    pub fn toggle(self, flags: &mut u8) {
        if self.is_set(*flags) {
            self.unset(flags)
        } else {
            self.set(flags)
        }
    }
}

/// The dynamic part of a dbus message header
#[derive(Debug, Clone, Default)]
pub struct DynamicHeader {
    pub interface: Option<String>,
    pub member: Option<String>,
    pub object: Option<String>,
    pub destination: Option<String>,
    pub serial: Option<u32>,
    pub sender: Option<String>,
    pub signature: Option<String>,
    pub error_name: Option<String>,
    pub response_serial: Option<u32>,
    pub num_fds: Option<u32>,
}

impl DynamicHeader {
    /// Make a correctly addressed error response with the correct response serial
    pub fn make_error_response<S: Into<String>>(
        &self,
        error_name: S,
        error_msg: Option<String>,
    ) -> crate::message_builder::MarshalledMessage {
        let mut err_resp = crate::message_builder::MarshalledMessage {
            typ: MessageType::Reply,
            dynheader: DynamicHeader {
                interface: None,
                member: None,
                object: None,
                destination: self.sender.clone(),
                serial: None,
                num_fds: None,
                sender: None,
                signature: None,
                response_serial: self.serial,
                error_name: Some(error_name.into()),
            },
            flags: 0,
            body: crate::message_builder::MarshalledMessageBody::new(),
        };
        if let Some(text) = error_msg {
            err_resp.body.push_param(text).unwrap();
        }
        err_resp
    }
    /// Make a correctly addressed response with the correct response serial
    pub fn make_response(&self) -> crate::message_builder::MarshalledMessage {
        crate::message_builder::MarshalledMessage {
            typ: MessageType::Reply,
            dynheader: DynamicHeader {
                interface: None,
                member: None,
                object: None,
                destination: self.sender.clone(),
                serial: None,
                num_fds: None,
                sender: None,
                signature: None,
                response_serial: self.serial,
                error_name: None,
            },
            flags: 0,
            body: crate::message_builder::MarshalledMessageBody::new(),
        }
    }
}

/// Starting point for new messages. Create either a call or a signal
#[derive(Default)]
pub struct MessageBuilder {
    msg: MarshalledMessage,
}

/// Created by MessageBuilder::call. Use it to make a new call to a service
pub struct CallBuilder {
    msg: MarshalledMessage,
}

/// Created by MessageBuilder::signal. Use it to make a new signal
pub struct SignalBuilder {
    msg: MarshalledMessage,
}

impl MessageBuilder {
    /// New messagebuilder with the default native byteorder
    pub fn new() -> MessageBuilder {
        MessageBuilder {
            msg: MarshalledMessage::new(),
        }
    }

    /// New messagebuilder with a chosen byteorder
    pub fn with_byteorder(b: ByteOrder) -> MessageBuilder {
        MessageBuilder {
            msg: MarshalledMessage::with_byteorder(b),
        }
    }

    pub fn call<S: Into<String>>(mut self, member: S) -> CallBuilder {
        self.msg.typ = MessageType::Call;
        self.msg.dynheader.member = Some(member.into());
        CallBuilder { msg: self.msg }
    }
    pub fn signal<S1, S2, S3>(mut self, interface: S1, member: S2, object: S3) -> SignalBuilder
    where
        S1: Into<String>,
        S2: Into<String>,
        S3: Into<String>,
    {
        self.msg.typ = MessageType::Signal;
        self.msg.dynheader.member = Some(member.into());
        self.msg.dynheader.interface = Some(interface.into());
        self.msg.dynheader.object = Some(object.into());
        SignalBuilder { msg: self.msg }
    }
}

impl CallBuilder {
    pub fn on<S: Into<String>>(mut self, object_path: S) -> Self {
        self.msg.dynheader.object = Some(object_path.into());
        self
    }

    pub fn with_interface<S: Into<String>>(mut self, interface: S) -> Self {
        self.msg.dynheader.interface = Some(interface.into());
        self
    }

    pub fn at<S: Into<String>>(mut self, destination: S) -> Self {
        self.msg.dynheader.destination = Some(destination.into());
        self
    }

    pub fn build(self) -> MarshalledMessage {
        self.msg
    }
}

impl SignalBuilder {
    pub fn to<S: Into<String>>(mut self, destination: S) -> Self {
        self.msg.dynheader.destination = Some(destination.into());
        self
    }

    pub fn build(self) -> MarshalledMessage {
        self.msg
    }
}

/// Message received by a connection or in preparation before being sent over a connection.
///
/// This represents a message while it is being built before it is sent over the connection.
/// The body accepts everything that implements the Marshal trait (e.g. all basic types, strings, slices, Hashmaps,.....)
/// And you can of course write an Marshal impl for your own datastructures. See the doc on the Marshal trait what you have
/// to look out for when doing this though.
#[derive(Debug)]
pub struct MarshalledMessage {
    pub body: MarshalledMessageBody,

    pub dynheader: DynamicHeader,

    pub typ: MessageType,
    pub flags: u8,
}

impl Default for MarshalledMessage {
    fn default() -> Self {
        Self::new()
    }
}

impl MarshalledMessage {
    pub fn get_buf(&self) -> &[u8] {
        &self.body.buf
    }
    pub fn get_sig(&self) -> &str {
        &self.body.sig
    }

    /// New message with the default native byteorder
    pub fn new() -> Self {
        MarshalledMessage {
            typ: MessageType::Invalid,
            dynheader: DynamicHeader::default(),

            flags: 0,
            body: MarshalledMessageBody::new(),
        }
    }

    /// New messagebody with a chosen byteorder
    pub fn with_byteorder(b: ByteOrder) -> Self {
        MarshalledMessage {
            typ: MessageType::Invalid,
            dynheader: DynamicHeader::default(),

            flags: 0,
            body: MarshalledMessageBody::with_byteorder(b),
        }
    }

    /// Reserves space for `additional` bytes in the internal buffer. This is useful to reduce the amount of allocations done while marshalling,
    /// if you can predict somewhat accuratly how many bytes you will be marshalling.
    pub fn reserve(&mut self, additional: usize) {
        self.body.reserve(additional)
    }

    pub fn unmarshall_all<'a, 'e>(self) -> Result<message::Message<'a, 'e>, UnmarshalError> {
        let params = if self.body.sig.is_empty() {
            vec![]
        } else {
            let sigs: Vec<_> = crate::signature::Type::parse_description(&self.body.sig)?;

            let (_, params) = crate::wire::unmarshal::unmarshal_body(
                self.body.byteorder,
                &sigs,
                &self.body.buf,
                &self.body.raw_fds,
                0,
            )?;
            params
        };
        Ok(message::Message {
            dynheader: self.dynheader,
            params,
            typ: self.typ,
            flags: self.flags,
            raw_fds: self.body.raw_fds,
        })
    }
}
/// The body accepts everything that implements the Marshal trait (e.g. all basic types, strings, slices, Hashmaps,.....)
/// And you can of course write an Marshal impl for your own datastrcutures
#[derive(Debug)]
pub struct MarshalledMessageBody {
    pub(crate) buf: Vec<u8>,

    // out of band data
    pub(crate) raw_fds: Vec<crate::wire::UnixFd>,

    sig: SignatureBuffer,
    pub(crate) byteorder: ByteOrder,
}

impl Default for MarshalledMessageBody {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function you might need, if the dbus API you use has Variants somewhere inside nested structures. If the the
/// API has a Variant at the top-level you can use MarshalledMessageBody::push_variant.
pub fn marshal_as_variant<P: Marshal>(
    p: P,
    byteorder: ByteOrder,
    buf: &mut Vec<u8>,
    fds: &mut Vec<crate::wire::UnixFd>,
) -> Result<(), MarshalError> {
    let mut ctx = MarshalContext {
        fds,
        buf,
        byteorder,
    };
    let ctx = &mut ctx;

    // get signature string and write it to the buffer
    let mut sig_str = SignatureBuffer::new();
    P::sig_str(&mut sig_str);
    let sig = crate::wire::SignatureWrapper::new(sig_str)?;
    sig.marshal(ctx)?;

    // the write the value to the buffer
    p.marshal(ctx)?;
    Ok(())
}

impl MarshalledMessageBody {
    /// New messagebody with the default native byteorder
    pub fn new() -> Self {
        MarshalledMessageBody {
            buf: Vec::new(),
            raw_fds: Vec::new(),
            sig: SignatureBuffer::new(),
            byteorder: ByteOrder::NATIVE,
        }
    }

    /// New messagebody with a chosen byteorder
    pub fn with_byteorder(b: ByteOrder) -> Self {
        MarshalledMessageBody {
            buf: Vec::new(),
            raw_fds: Vec::new(),
            sig: SignatureBuffer::new(),
            byteorder: b,
        }
    }

    pub fn from_parts(
        buf: Vec<u8>,
        raw_fds: Vec<crate::wire::UnixFd>,
        sig: String,
        byteorder: ByteOrder,
    ) -> Self {
        let sig = SignatureBuffer::from_string(sig);
        Self {
            buf,
            raw_fds,
            sig,
            byteorder,
        }
    }
    /// Get a clone of all the `UnixFd`s in the body.
    ///
    /// Some of the `UnixFd`s may already have their `RawFd`s taken.
    pub fn get_fds(&self) -> Vec<UnixFd> {
        self.raw_fds.clone()
    }
    /// Clears the buffer and signature but holds on to the memory allocations. You can now start pushing new
    /// params as if this were a new message. This allows to reuse the OutMessage for the same dbus-message with different
    /// parameters without allocating the buffer every time.
    pub fn reset(&mut self) {
        self.sig.clear();
        self.buf.clear();
    }

    /// Reserves space for `additional` bytes in the internal buffer. This is useful to reduce the amount of allocations done while marshalling,
    /// if you can predict somewhat accuratly how many bytes you will be marshalling.
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional)
    }

    /// Push a Param with the old nested enum/struct approach. This is still supported for the case that in some corner cases
    /// the new trait/type based API does not work.
    pub fn push_old_param(&mut self, p: &crate::params::Param) -> Result<(), MarshalError> {
        let mut ctx = MarshalContext {
            buf: &mut self.buf,
            fds: &mut self.raw_fds,
            byteorder: self.byteorder,
        };
        let ctx = &mut ctx;
        crate::wire::marshal::container::marshal_param(p, ctx)?;
        p.sig().to_str(self.sig.to_string_mut());
        Ok(())
    }

    /// Convenience function to call push_old_param on a slice of Param
    pub fn push_old_params(&mut self, ps: &[crate::params::Param]) -> Result<(), MarshalError> {
        for p in ps {
            self.push_old_param(p)?;
        }
        Ok(())
    }
    fn create_ctx(&mut self) -> MarshalContext {
        MarshalContext {
            buf: &mut self.buf,
            fds: &mut self.raw_fds,
            byteorder: self.byteorder,
        }
    }

    /// Append something that is Marshal to the message body
    pub fn push_param<P: Marshal>(&mut self, p: P) -> Result<(), MarshalError> {
        let mut ctx = self.create_ctx();
        p.marshal(&mut ctx)?;
        P::sig_str(&mut self.sig);
        Ok(())
    }

    /// execute some amount of push calls and if any of them fails, reset the body
    // to the state it was in before the push calls where executed
    fn push_mult_helper<F>(&mut self, push_calls: F) -> Result<(), MarshalError>
    where
        F: FnOnce(&mut MarshalledMessageBody) -> Result<(), MarshalError>,
    {
        let sig_len = self.sig.len();
        let buf_len = self.buf.len();
        let fds_len = self.raw_fds.len();

        match push_calls(self) {
            Ok(ret) => Ok(ret),
            Err(e) => {
                // reset state to before any of the push calls happened
                self.sig.truncate(sig_len)?;
                self.buf.truncate(buf_len);
                self.raw_fds.truncate(fds_len);
                Err(e)
            }
        }
    }

    /// Append two things that are Marshal to the message body
    pub fn push_param2<P1: Marshal, P2: Marshal>(
        &mut self,
        p1: P1,
        p2: P2,
    ) -> Result<(), MarshalError> {
        self.push_mult_helper(move |msg: &mut Self| {
            msg.push_param(p1)?;
            msg.push_param(p2)
        })
    }

    /// Append three things that are Marshal to the message body
    pub fn push_param3<P1: Marshal, P2: Marshal, P3: Marshal>(
        &mut self,
        p1: P1,
        p2: P2,
        p3: P3,
    ) -> Result<(), MarshalError> {
        self.push_mult_helper(move |msg: &mut Self| {
            msg.push_param(p1)?;
            msg.push_param(p2)?;
            msg.push_param(p3)
        })
    }

    /// Append four things that are Marshal to the message body
    pub fn push_param4<P1: Marshal, P2: Marshal, P3: Marshal, P4: Marshal>(
        &mut self,
        p1: P1,
        p2: P2,
        p3: P3,
        p4: P4,
    ) -> Result<(), MarshalError> {
        self.push_mult_helper(move |msg: &mut Self| {
            msg.push_param(p1)?;
            msg.push_param(p2)?;
            msg.push_param(p3)?;
            msg.push_param(p4)
        })
    }

    /// Append five things that are Marshal to the message body
    pub fn push_param5<P1: Marshal, P2: Marshal, P3: Marshal, P4: Marshal, P5: Marshal>(
        &mut self,
        p1: P1,
        p2: P2,
        p3: P3,
        p4: P4,
        p5: P5,
    ) -> Result<(), MarshalError> {
        self.push_mult_helper(move |msg: &mut Self| {
            msg.push_param(p1)?;
            msg.push_param(p2)?;
            msg.push_param(p3)?;
            msg.push_param(p4)?;
            msg.push_param(p5)
        })
    }

    /// Append any number of things that have the same type that is Marshal to the message body
    pub fn push_params<P: Marshal>(&mut self, params: &[P]) -> Result<(), MarshalError> {
        for p in params {
            self.push_param(p)?;
        }
        Ok(())
    }

    /// Append something that is Marshal to the body but use a dbus Variant in the signature. This is necessary for some APIs
    pub fn push_variant<P: Marshal>(&mut self, p: P) -> Result<(), MarshalError> {
        self.sig.push_static("v");
        let mut ctx = self.create_ctx();
        p.marshal_as_variant(&mut ctx)
    }
    /// Validate the all the marshalled elements of the body.
    pub fn validate(&self) -> Result<(), UnmarshalError> {
        if self.sig.is_empty() && self.buf.is_empty() {
            return Ok(());
        }
        let types = crate::signature::Type::parse_description(&self.sig)?;
        let mut used = 0;
        for typ in types {
            used += validate_raw::validate_marshalled(self.byteorder, used, &self.buf, &typ)
                .map_err(|(_, e)| e)?;
        }
        if used == self.buf.len() {
            Ok(())
        } else {
            Err(UnmarshalError::NotAllBytesUsed)
        }
    }
    /// Create a parser to retrieve parameters from the body.
    #[inline]
    pub fn parser(&self) -> MessageBodyParser {
        MessageBodyParser::new(self)
    }
}

#[test]
fn test_marshal_trait() {
    let mut body = MarshalledMessageBody::new();
    let bytes: &[&[_]] = &[&[4u64]];
    body.push_param(bytes).unwrap();

    assert_eq!(
        vec![12, 0, 0, 0, 8, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0],
        body.buf
    );
    assert_eq!(body.sig.as_str(), "aat");

    let mut body = MarshalledMessageBody::new();
    let mut map = std::collections::HashMap::new();
    map.insert("a", 4u32);

    body.push_param(&map).unwrap();
    assert_eq!(
        vec![12, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'a', 0, 0, 0, 4, 0, 0, 0,],
        body.buf
    );
    assert_eq!(body.sig.as_str(), "a{su}");

    let mut body = MarshalledMessageBody::new();
    body.push_param((11u64, "str", true)).unwrap();
    assert_eq!(body.sig.as_str(), "(tsb)");
    assert_eq!(
        vec![11, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, b's', b't', b'r', 0, 1, 0, 0, 0,],
        body.buf
    );

    struct MyStruct {
        x: u64,
        y: String,
    }

    use crate::wire::marshal::traits::Signature;
    use crate::wire::marshal::MarshalContext;
    impl Signature for &MyStruct {
        fn signature() -> crate::signature::Type {
            crate::signature::Type::Container(crate::signature::Container::Struct(
                crate::signature::StructTypes::new(vec![u64::signature(), String::signature()])
                    .unwrap(),
            ))
        }

        fn alignment() -> usize {
            8
        }
        #[inline]
        fn sig_str(s_buf: &mut crate::wire::marshal::traits::SignatureBuffer) {
            s_buf.push_static("(ts)")
        }
        fn has_sig(sig: &str) -> bool {
            sig == "(ts)"
        }
    }
    impl Marshal for &MyStruct {
        fn marshal(&self, ctx: &mut MarshalContext) -> Result<(), MarshalError> {
            // always align to 8
            ctx.align_to(8);
            self.x.marshal(ctx)?;
            self.y.marshal(ctx)?;
            Ok(())
        }
    }

    let mut body = MarshalledMessageBody::new();
    body.push_param(&MyStruct {
        x: 100,
        y: "A".to_owned(),
    })
    .unwrap();
    assert_eq!(body.sig.as_str(), "(ts)");
    assert_eq!(
        vec![100, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'A', 0,],
        body.buf
    );

    let mut body = MarshalledMessageBody::new();
    let emptymap: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut map = std::collections::HashMap::new();
    let mut map2 = std::collections::HashMap::new();
    map.insert("a", 4u32);
    map2.insert("a", &map);

    body.push_param(&map2).unwrap();
    body.push_param(&emptymap).unwrap();
    assert_eq!(body.sig.as_str(), "a{sa{su}}a{su}");
    assert_eq!(
        vec![
            28, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'a', 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0,
            0, b'a', 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0
        ],
        body.buf
    );

    // try to unmarshal stuff
    let mut body_iter = MessageBodyParser::new(&body);

    // first try some stuff that has the wrong signature
    type WrongNestedDict =
        std::collections::HashMap<String, std::collections::HashMap<String, u64>>;
    assert_eq!(
        body_iter.get::<WrongNestedDict>().err().unwrap(),
        UnmarshalError::WrongSignature
    );
    type WrongStruct = (u64, i32, String);
    assert_eq!(
        body_iter.get::<WrongStruct>().err().unwrap(),
        UnmarshalError::WrongSignature
    );

    // the get the correct type and make sure the content is correct
    type NestedDict = std::collections::HashMap<String, std::collections::HashMap<String, u32>>;
    let newmap2: NestedDict = body_iter.get().unwrap();
    assert_eq!(newmap2.len(), 1);
    assert_eq!(newmap2.get("a").unwrap().len(), 1);
    assert_eq!(*newmap2.get("a").unwrap().get("a").unwrap(), 4);

    // again try some stuff that has the wrong signature
    assert_eq!(
        body_iter.get::<WrongNestedDict>().err().unwrap(),
        UnmarshalError::WrongSignature
    );
    assert_eq!(
        body_iter.get::<WrongStruct>().err().unwrap(),
        UnmarshalError::WrongSignature
    );

    // get the empty map next
    let newemptymap: std::collections::HashMap<&str, u32> = body_iter.get().unwrap();
    assert_eq!(newemptymap.len(), 0);

    // test get2()
    let mut body_iter = body.parser();
    assert_eq!(
        body_iter.get2::<NestedDict, u16>().unwrap_err(),
        UnmarshalError::WrongSignature
    );
    assert_eq!(
        body_iter
            .get3::<NestedDict, std::collections::HashMap<&str, u32>, u32>()
            .unwrap_err(),
        UnmarshalError::EndOfMessage
    );

    // test to make sure body_iter is left unchanged from last failure and the map is
    // pulled out identically from above
    let (newmap2, newemptymap): (NestedDict, std::collections::HashMap<&str, u32>) =
        body_iter.get2().unwrap();
    // repeat assertions from above
    assert_eq!(newmap2.len(), 1);
    assert_eq!(newmap2.get("a").unwrap().len(), 1);
    assert_eq!(*newmap2.get("a").unwrap().get("a").unwrap(), 4);
    assert_eq!(newemptymap.len(), 0);
    assert_eq!(
        body_iter.get::<u16>().unwrap_err(),
        UnmarshalError::EndOfMessage
    );

    // test mixed get() and get_param()
    let mut body_iter = body.parser();

    // test to make sure body_iter is left unchanged from last failure and the map is
    // pulled out identically from above
    let newmap2: NestedDict = body_iter.get().unwrap();
    let newemptymap = body_iter.get_param().unwrap();
    // repeat assertions from above
    assert_eq!(newmap2.len(), 1);
    assert_eq!(newmap2.get("a").unwrap().len(), 1);
    assert_eq!(*newmap2.get("a").unwrap().get("a").unwrap(), 4);

    use crate::params::Container;
    use crate::params::Param;
    match newemptymap {
        Param::Container(Container::Dict(dict)) => {
            assert_eq!(dict.map.len(), 0);
            assert_eq!(dict.key_sig, crate::signature::Base::String);
            assert_eq!(
                dict.value_sig,
                crate::signature::Type::Base(crate::signature::Base::Uint32)
            );
        }
        _ => panic!("Expected to get a dict"),
    }
    assert_eq!(
        body_iter.get::<u16>().unwrap_err(),
        UnmarshalError::EndOfMessage
    );
}

use crate::wire::unmarshal::traits::Unmarshal;
/// Iterate over the messages parameters
///
/// Because dbus allows for multiple toplevel params without an enclosing struct, this provides a simple Iterator (sadly not std::iterator::Iterator, since the types
/// of the parameters can be different)
/// that you can use to get the params one by one, calling `get::<T>` until you have obtained all the parameters.
/// If you try to get more parameters than the signature has types, it will return None, if you try to get a parameter that doesn not
/// fit the current one, it will return an Error::WrongSignature, but you can safely try other types, the iterator stays valid.
#[derive(Debug)]
pub struct MessageBodyParser<'body> {
    buf_idx: usize,
    sig_idx: usize,
    body: &'body MarshalledMessageBody,
}

impl<'fds, 'body: 'fds> MessageBodyParser<'body> {
    pub fn new(body: &'body MarshalledMessageBody) -> Self {
        Self {
            buf_idx: 0,
            sig_idx: 0,
            body,
        }
    }

    #[inline(always)]
    fn sig_iter(&self) -> SignatureIter<'body> {
        SignatureIter::new_at_idx(self.body.sig.as_str(), self.sig_idx)
    }

    /// Get the next params signature (if any are left)
    #[inline(always)]
    pub fn get_next_sig(&self) -> Option<&'body str> {
        self.sig_iter().next()
    }

    #[inline(always)]
    pub fn sigs_left(&self) -> usize {
        self.sig_iter().count()
    }

    /// Get the next param, use get::<TYPE> to specify what type you expect. For example `let s = parser.get::<String>()?;`
    /// This checks if there are params left in the message and if the type you requested fits the signature of the message.
    pub fn get<T: Unmarshal<'body, 'fds>>(&mut self) -> Result<T, UnmarshalError> {
        if let Some(expected_sig) = self.get_next_sig() {
            if !T::has_sig(expected_sig) {
                return Err(UnmarshalError::WrongSignature);
            }

            let mut ctx = UnmarshalContext {
                byteorder: self.body.byteorder,
                buf: &self.body.buf,
                offset: self.buf_idx,
                fds: &self.body.raw_fds,
            };
            match T::unmarshal(&mut ctx) {
                Ok((bytes, res)) => {
                    self.buf_idx += bytes;
                    self.sig_idx += expected_sig.len();
                    Ok(res)
                }
                Err(e) => Err(e),
            }
        } else {
            Err(UnmarshalError::EndOfMessage)
        }
    }
    /// Perform error handling for `get2(), get3()...` if `get_calls` fails.
    fn get_mult_helper<T, F>(&mut self, count: usize, get_calls: F) -> Result<T, UnmarshalError>
    where
        F: FnOnce(&mut Self) -> Result<T, UnmarshalError>,
    {
        if count > self.sigs_left() {
            return Err(UnmarshalError::EndOfMessage);
        }
        let start_sig_idx = self.sig_idx;
        let start_buf_idx = self.buf_idx;
        match get_calls(self) {
            Ok(ret) => Ok(ret),
            Err(err) => {
                self.sig_idx = start_sig_idx;
                self.buf_idx = start_buf_idx;
                Err(err)
            }
        }
    }

    /// Get the next two params, use get2::<TYPE, TYPE> to specify what type you expect. For example `let s = parser.get2::<String, i32>()?;`
    /// This checks if there are params left in the message and if the type you requested fits the signature of the message.
    pub fn get2<T1, T2>(&mut self) -> Result<(T1, T2), UnmarshalError>
    where
        T1: Unmarshal<'body, 'fds>,
        T2: Unmarshal<'body, 'fds>,
    {
        let get_calls = |parser: &mut Self| {
            let ret1 = parser.get()?;
            let ret2 = parser.get()?;
            Ok((ret1, ret2))
        };
        self.get_mult_helper(2, get_calls)
    }

    /// Get the next three params, use get3::<TYPE, TYPE, TYPE> to specify what type you expect. For example `let s = parser.get3::<String, i32, u64>()?;`
    /// This checks if there are params left in the message and if the type you requested fits the signature of the message.
    pub fn get3<T1, T2, T3>(&mut self) -> Result<(T1, T2, T3), UnmarshalError>
    where
        T1: Unmarshal<'body, 'fds>,
        T2: Unmarshal<'body, 'fds>,
        T3: Unmarshal<'body, 'fds>,
    {
        let get_calls = |parser: &mut Self| {
            let ret1 = parser.get()?;
            let ret2 = parser.get()?;
            let ret3 = parser.get()?;
            Ok((ret1, ret2, ret3))
        };
        self.get_mult_helper(3, get_calls)
    }

    /// Get the next four params, use get4::<TYPE, TYPE, TYPE, TYPE> to specify what type you expect. For example `let s = parser.get4::<String, i32, u64, u8>()?;`
    /// This checks if there are params left in the message and if the type you requested fits the signature of the message.
    pub fn get4<T1, T2, T3, T4>(&mut self) -> Result<(T1, T2, T3, T4), UnmarshalError>
    where
        T1: Unmarshal<'body, 'fds>,
        T2: Unmarshal<'body, 'fds>,
        T3: Unmarshal<'body, 'fds>,
        T4: Unmarshal<'body, 'fds>,
    {
        let get_calls = |parser: &mut Self| {
            let ret1 = parser.get()?;
            let ret2 = parser.get()?;
            let ret3 = parser.get()?;
            let ret4 = parser.get()?;
            Ok((ret1, ret2, ret3, ret4))
        };
        self.get_mult_helper(4, get_calls)
    }

    /// Get the next five params, use get5::<TYPE, TYPE, TYPE, TYPE, TYPE> to specify what type you expect. For example `let s = parser.get4::<String, i32, u64, u8, bool>()?;`
    /// This checks if there are params left in the message and if the type you requested fits the signature of the message.
    pub fn get5<T1, T2, T3, T4, T5>(&mut self) -> Result<(T1, T2, T3, T4, T5), UnmarshalError>
    where
        T1: Unmarshal<'body, 'fds>,
        T2: Unmarshal<'body, 'fds>,
        T3: Unmarshal<'body, 'fds>,
        T4: Unmarshal<'body, 'fds>,
        T5: Unmarshal<'body, 'fds>,
    {
        let get_calls = |parser: &mut Self| {
            let ret1 = parser.get()?;
            let ret2 = parser.get()?;
            let ret3 = parser.get()?;
            let ret4 = parser.get()?;
            let ret5 = parser.get()?;
            Ok((ret1, ret2, ret3, ret4, ret5))
        };
        self.get_mult_helper(5, get_calls)
    }

    /// Get the next (old_style) param.
    /// This checks if there are params left in the message and if the type you requested fits the signature of the message.
    pub fn get_param(&mut self) -> Result<crate::params::Param, UnmarshalError> {
        if let Some(sig_str) = self.get_next_sig() {
            let mut ctx = UnmarshalContext {
                byteorder: self.body.byteorder,
                buf: &self.body.buf,
                offset: self.buf_idx,
                fds: &self.body.raw_fds,
            };

            let sig = &crate::signature::Type::parse_description(sig_str).unwrap()[0];

            match crate::wire::unmarshal::container::unmarshal_with_sig(sig, &mut ctx) {
                Ok((bytes, res)) => {
                    self.buf_idx += bytes;
                    self.sig_idx += sig_str.len();
                    Ok(res)
                }
                Err(e) => Err(e),
            }
        } else {
            Err(UnmarshalError::EndOfMessage)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parser_get() {
        use crate::wire::errors::UnmarshalError;

        let mut sig = super::MessageBuilder::new()
            .signal("io.killingspark", "Signal", "/io/killingspark/Signaler")
            .build();

        sig.body.push_param3(100u32, 200i32, "ABCDEFGH").unwrap();

        let mut parser = sig.body.parser();
        assert_eq!(parser.get(), Ok(100u32));
        assert_eq!(parser.get(), Ok(200i32));
        assert_eq!(parser.get(), Ok("ABCDEFGH"));
        assert_eq!(parser.get::<String>(), Err(UnmarshalError::EndOfMessage));

        let mut parser = sig.body.parser();
        assert_eq!(parser.get2(), Ok((100u32, 200i32)));
        assert_eq!(parser.get(), Ok("ABCDEFGH"));
        assert_eq!(parser.get::<String>(), Err(UnmarshalError::EndOfMessage));

        let mut parser = sig.body.parser();
        assert_eq!(parser.get3(), Ok((100u32, 200i32, "ABCDEFGH")));
        assert_eq!(parser.get::<String>(), Err(UnmarshalError::EndOfMessage));

        let mut sig = super::MessageBuilder::new()
            .signal("io.killingspark", "Signal", "/io/killingspark/Signaler")
            .build();

        sig.body.push_param((100u32, 200i32, "ABCDEFGH")).unwrap();
        sig.body.push_param((100u32, 200i32, "ABCDEFGH")).unwrap();
        sig.body.push_param((100u32, 200i32, "ABCDEFGH")).unwrap();

        let mut parser = sig.body.parser();
        assert!(parser.get::<(u32, i32, &str)>().is_ok());
        assert!(parser.get2::<(u32, i32, &str), (u32, i32, &str)>().is_ok());
    }
}
