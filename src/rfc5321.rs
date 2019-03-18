//! Parser for SMTP syntax.

use std::fmt::{self, Display};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::{self, FromStr};

use nom::{is_alphanumeric, is_digit, is_hex_digit};

use crate::util::*;
use crate::rfc5234::wsp;
use crate::rfc5322::{atext as atom};

#[derive(Clone, Debug, PartialEq)]
pub struct EsmtpParam(pub String, pub Option<String>);

/// Represents a forward path from the `"RCPT TO"` command.
#[derive(Clone, Debug, PartialEq)]
pub enum Path {
    /// RCPT TO: \<person@example.org\>
    Mailbox(Mailbox),
    /// RCPT TO: \<postmaster\>
    PostMaster,
}

/// Represents a reverse path from the `"MAIL FROM"` command.
#[derive(Clone, Debug, PartialEq)]
pub enum ReversePath {
    /// MAIL FROM: \<person@example.org\>
    Mailbox(Mailbox),
    /// MAIL FROM: \<\>
    Null,
}

/// The local part of an address preceding the `"@"` in an email address.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalPart {
    Atom(String),
    Quoted(String),
}

impl Display for LocalPart {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LocalPart::Atom(a) => write!(f, "{}", a),
            LocalPart::Quoted(q) => write!(f, "{}", quote_localpart(q)),
        }
    }
}

fn quote_localpart(input: &str) -> String {
    let mut out = String::with_capacity(input.len());

    for c in input.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c)
        }
    }

    out
}

/// The domain part of an address following the `"@"` in an email address.
#[derive(Clone, Debug, PartialEq)]
pub enum DomainPart {
    /// A DNS domain name such as `"example.org"`.
    Domain(String),
    /// An address literal.
    AddressLiteral(AddressLiteral),
}

impl Display for DomainPart {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DomainPart::Domain(d) => write!(f, "{}", d),
            DomainPart::AddressLiteral(a) => write!(f, "{}", a),
        }
    }
}

/// An SMTP network address literal.
#[derive(Clone, Debug, PartialEq)]
pub enum AddressLiteral {
    /// An IPv4 or IPv6 address literal.
    /// # Examples
    /// ```
    /// use std::net::{Ipv4Addr, Ipv6Addr};
    /// use rustyknife::rfc5321::AddressLiteral;
    ///
    /// let ipv4 : AddressLiteral = "[192.0.2.1]".parse().unwrap();
    /// let ipv6 : AddressLiteral = "[IPv6:2001:db8::1]".parse().unwrap();
    ///
    /// assert_eq!(ipv4, AddressLiteral::IpAddr("192.0.2.1".parse().unwrap()));
    /// assert_eq!(ipv6, AddressLiteral::IpAddr("2001:db8::1".parse().unwrap()));
    /// ```
    IpAddr(IpAddr),
    /// An address literal in the form tag:value.
    /// # Examples
    /// ```
    /// use rustyknife::rfc5321::AddressLiteral;
    ///
    /// let lit : AddressLiteral = "[x400:cn=bob,dc=example,dc=org]".parse().unwrap();
    /// assert_eq!(lit, AddressLiteral::Tagged("x400".into(), "cn=bob,dc=example,dc=org".into()));
    /// ```
    Tagged(String, String),
    /// A free form address literal. Generated by the RFC5322 code.
    FreeForm(String),
}

impl AddressLiteral {
    /// Try to upgrade a [`AddressLiteral::FreeForm`] to the more formal subtypes.
    /// # Examples
    /// ```
    /// use rustyknife::rfc5321::AddressLiteral;
    ///
    /// let valid = AddressLiteral::FreeForm("192.0.2.1".into());
    /// let invalid = AddressLiteral::FreeForm("somewhere".into());
    ///
    /// assert_eq!(valid.upgrade(), Ok(AddressLiteral::IpAddr("192.0.2.1".parse().unwrap())));
    /// assert_eq!(invalid.upgrade(), Err(()));
    /// ```
    pub fn upgrade(&self) -> Result<Self, ()> {
        if let AddressLiteral::FreeForm(s) = self {
            let (rem, parsed) = _inner_address_literal(CBS(s.as_bytes())).map_err(|_| ())?;

            if rem.is_empty() {
                Ok(parsed)
            } else {
                Err(())
            }
        } else {
            Err(())
        }
    }
}

impl FromStr for AddressLiteral {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (rem, parsed) = address_literal(CBS(s.as_bytes())).map_err(|_| ())?;
        if rem.is_empty() {
            Ok(parsed)
        } else {
            Err(())
        }
    }
}

impl Display for AddressLiteral {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AddressLiteral::IpAddr(ip) => match ip {
                IpAddr::V4(ipv4) => write!(f, "[{}]", ipv4),
                IpAddr::V6(ipv6) => write!(f, "[IPv6:{}]", ipv6),
            },
            AddressLiteral::Tagged(tag, value) => write!(f, "[{}:{}]", tag, value),
            AddressLiteral::FreeForm(value) => write!(f, "[{}]", value),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mailbox(pub LocalPart, pub DomainPart);

impl Display for Mailbox {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}@{}", self.0, self.1)
    }
}

#[inline]
named!(_alphanum<CBS, CBS>,
    verify!(take!(1), |x: CBS| is_alphanumeric(x.0[0]))
);

named!(esmtp_keyword<CBS, &'_ str>,
    map!(recognize!(do_parse!(_alphanum >> many0!(_alphanum) >> ())), |x| std::str::from_utf8(&x).unwrap())
);

named!(esmtp_value<CBS, &'_ str>,
    map!(take_while1!(|c| (33..=60).contains(&c) || (62..=126).contains(&c)),
         |x| std::str::from_utf8(&x).unwrap())
);

named!(esmtp_param<CBS, EsmtpParam>,
    do_parse!(
        name: esmtp_keyword >>
        value: opt!(do_parse!(tag!("=") >>  v: esmtp_value >> (v))) >>
        (EsmtpParam(name.into(), value.map(|v| v.into())))
    )
);

named!(_esmtp_params<CBS, Vec<EsmtpParam>>,
    do_parse!(
        a: esmtp_param >>
        b: many0!(do_parse!(many1!(wsp) >> c: esmtp_param >> (c))) >>
        ({ let mut out = Vec::with_capacity(b.len()+1); out.push(a); out.extend_from_slice(&b); out })
    )
);

named!(ldh_str<CBS, CBS>,
    verify!(take_while1!(|c| is_alphanumeric(c) || c == b'-'), |x: CBS| {
        x.0.last() != Some(&b'-')
    })
);

#[inline]
named!(let_dig<CBS, CBS>,
    verify!(take!(1), |c: CBS| is_alphanumeric(c.0[0]))
);

named!(sub_domain<CBS, CBS>,
    recognize!(do_parse!(
        let_dig >>
        opt!(ldh_str) >>
        ()
    ))
);

named!(domain<CBS, DomainPart>,
    map!(recognize!(do_parse!(sub_domain >> many0!(do_parse!(tag!(".") >> sub_domain >> ())) >> ())),
         |domain| DomainPart::Domain(ascii_to_string(domain).into())
    )
);

named!(at_domain<CBS, ()>,
    do_parse!(
        tag!("@") >>
        domain >>
        ()
    )
);

named!(a_d_l<CBS, ()>,
    do_parse!(
        at_domain >>
        many0!(do_parse!(tag!(",") >> at_domain >> ())) >>
        ()
    )
);

named!(dot_string<CBS, CBS>,
    recognize!(do_parse!(
        atom >>
        many0!(do_parse!(tag!(".") >> atom >> ())) >>
        ()
    ))
);

#[inline]
named!(qtext_smtp<CBS, char>,
   map!(verify!(take!(1), |x: CBS| {
       let c = &x.0[0];
       (32..=33).contains(c) || (35..=91).contains(c) || (93..=126).contains(c)
   }), |x| x.0[0] as char)
);

#[inline]
named!(quoted_pair_smtp<CBS, char>,
    do_parse!(
        tag!("\\") >>
        c: map!(verify!(take!(1), |x: CBS| {
            let c = &x.0[0];
            (32..=126).contains(c)
        }), |x| x.0[0]) >>
        (c as char)
    )
);

named!(qcontent_smtp<CBS, char>,
    alt!(qtext_smtp | quoted_pair_smtp)
);

named!(quoted_string<CBS, String>,
    do_parse!(
        tag!("\"") >>
        qc: many0!(qcontent_smtp) >>
        tag!("\"") >>
        (qc.into_iter().collect())
    )
);

named!(local_part<CBS, LocalPart>,
    alt!(map!(dot_string, |s| LocalPart::Atom(ascii_to_string(s).into())) |
         map!(quoted_string, LocalPart::Quoted))
);

named!(_ip_int<CBS, u8>,
    map_res!(take_while_m_n!(1, 3, is_digit),
             |ip : CBS| str::from_utf8(ip.0).unwrap().parse()
    )
);

named!(_ipv4_literal<CBS, AddressLiteral>,
    do_parse!(
        a: _ip_int >>
        b: many_m_n!(3, 3, do_parse!(tag!(".") >> i: _ip_int >> (i))) >>
        (AddressLiteral::IpAddr(IpAddr::V4(Ipv4Addr::new(a, b[0], b[1], b[2]))))
    )
);

named!(_ipv6_literal<CBS, AddressLiteral>,
    map!(map_res!(do_parse!(
        tag_no_case!("IPv6:") >>
        addr: take_while1!(|c| is_hex_digit(c) || b":.".contains(&c))  >>
        (addr)),
        |addr : CBS| {
            Ipv6Addr::from_str(str::from_utf8(addr.0).unwrap())
        }),
        |addr| AddressLiteral::IpAddr(IpAddr::V6(addr)))
);

named!(dcontent<CBS, &'_ str>,
    map!(take_while1!(|c| (33..=90).contains(&c) || (94..=126).contains(&c)),
         |x| std::str::from_utf8(&x.0).unwrap())
);

named!(general_address_literal<CBS, AddressLiteral>,
    do_parse!(
        tag: ldh_str >>
        tag!(":") >>
        value: dcontent >>
        (AddressLiteral::Tagged(str::from_utf8(tag.0).unwrap().into(), value.into()))
    )
);

named!(_inner_address_literal<CBS, AddressLiteral>,
    alt!(_ipv4_literal | _ipv6_literal | general_address_literal)
);

named!(address_literal<CBS, AddressLiteral>,
    do_parse!(
        tag!("[") >>
        lit: _inner_address_literal >>
        tag!("]") >>
        (lit)
    )
);

named!(mailbox<CBS, Mailbox>,
    do_parse!(
        lp: local_part >>
        tag!("@") >>
        dp: alt!(domain | map!(address_literal, DomainPart::AddressLiteral)) >>
        (Mailbox(lp, dp))
    )
);

named!(path<CBS, Mailbox>,
    do_parse!(
        tag!("<") >>
        opt!(do_parse!(a_d_l >> tag!(":") >> ())) >>
        m: mailbox >>
        tag!(">") >>
        (m)
    )
);

named!(reverse_path<CBS, ReversePath>,
    alt!(map!(path, ReversePath::Mailbox) |
         map!(tag!("<>"), |_| ReversePath::Null))
);

named!(_mail_command<CBS, (ReversePath, Vec<EsmtpParam>)>,
    do_parse!(
        tag_no_case!("MAIL FROM:") >>
        addr: reverse_path >>
        params: opt!(do_parse!(tag!(" ") >> p: _esmtp_params >> (p))) >>
        (addr, params.unwrap_or_default())
    )
);

named!(_rcpt_command<CBS, (Path, Vec<EsmtpParam>)>,
    do_parse!(
        tag_no_case!("RCPT TO:") >>
        addr: alt!(
            map!(tag_no_case!("<postmaster>"), |_| Path::PostMaster) |
            map!(path, Path::Mailbox)
        ) >>
        params: opt!(do_parse!(tag!(" ") >> p: _esmtp_params >> (p))) >>
        (addr, params.unwrap_or_default())
    )
);

pub fn mail_command(i: &[u8]) -> KResult<&[u8], (ReversePath, Vec<EsmtpParam>)> {
    wrap_cbs_result(exact!(CBS(i), _mail_command))
}

pub fn rcpt_command(i: &[u8]) -> KResult<&[u8], (Path, Vec<EsmtpParam>)> {
    wrap_cbs_result(exact!(CBS(i), _rcpt_command))
}

/// Validates an email address.
/// Does not accept the empty address.
pub fn validate_address(i: &[u8]) -> bool {
    exact!(CBS(i), mailbox).is_ok()
}
