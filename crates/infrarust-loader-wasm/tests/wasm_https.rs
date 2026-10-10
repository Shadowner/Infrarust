#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/net.rs"]
mod net;
mod support;

use std::sync::Arc;

use net::{Acceptor, Duplex, HttpServer, enable_probe, network_toml};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

struct Tls {
    ca_pem: String,
    trusted: Acceptor,
    untrusted: Acceptor,
}

fn acceptor(cert: CertificateDer<'static>, key: &KeyPair) -> Acceptor {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    Arc::new(move |stream| {
        let acceptor = acceptor.clone();
        Box::pin(async move {
            acceptor
                .accept(stream)
                .await
                .ok()
                .map(|stream| Box::new(stream) as Box<dyn Duplex>)
        })
    })
}

fn leaf_params() -> CertificateParams {
    let mut params =
        CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params
}

fn certificates() -> Tls {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "infrarust test ca");
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);

    let leaf_key = KeyPair::generate().unwrap();
    let leaf = leaf_params().signed_by(&leaf_key, &issuer).unwrap();

    let stranger_key = KeyPair::generate().unwrap();
    let stranger = leaf_params().self_signed(&stranger_key).unwrap();

    Tls {
        ca_pem: ca_cert.pem(),
        trusted: acceptor(leaf.der().clone(), &leaf_key),
        untrusted: acceptor(stranger.der().clone(), &stranger_key),
    }
}

#[test]
fn https_reaches_a_trusted_allowed_server_and_nothing_else() {
    let tls = certificates();
    let dir = tempfile::tempdir().unwrap();
    let ca_file = dir.path().join("ca.pem");
    std::fs::write(&ca_file, &tls.ca_pem).unwrap();
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("SSL_CERT_FILE", &ca_file);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let trusted = HttpServer::start_with("secure hello", tls.trusted).await;
        let untrusted = HttpServer::start_with("impostor", tls.untrusted).await;
        let refused = HttpServer::start("never").await;
        let port = trusted.addr.port();
        let probe = enable_probe(
            &["network"],
            &network_toml(
                &[
                    format!("localhost:{port}"),
                    trusted.addr.to_string(),
                    untrusted.addr.to_string(),
                ],
                "",
            ),
        )
        .await;

        assert_eq!(
            probe
                .run(&format!("http https://localhost:{port}/secure"))
                .await,
            "ok 200 secure hello"
        );
        assert_eq!(
            probe.run(&format!("http https://{}/", trusted.addr)).await,
            "ok 200 secure hello"
        );
        let impostor = probe
            .run(&format!("http https://{}/", untrusted.addr))
            .await;
        assert!(impostor.contains("TlsCertificateError"), "{impostor}");
        let denied = probe.run(&format!("http https://{}/", refused.addr)).await;
        assert!(denied.contains("HttpRequestDenied"), "{denied}");

        assert_eq!(trusted.accepts_after_quiet().await, 2);
        let requests = trusted.requests().await;
        assert!(
            requests[0].starts_with("GET /secure HTTP/1.1\r\n"),
            "{requests:?}"
        );
        assert_eq!(untrusted.accepts_after_quiet().await, 1);
        assert!(untrusted.requests().await.is_empty());
        assert_eq!(refused.accepts_after_quiet().await, 0);
    });
}
