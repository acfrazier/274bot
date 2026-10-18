use script::load::{LoadIsolate, LoadShape};

#[test]
fn crypto_stub_keeps_uuid_and_random_values_without_subtle_or_url() {
    let isolate = LoadIsolate::spawn(
        r#"export default class CryptoStub extends LoopingBot { loop() {} }"#.into(),
        LoadShape::CompatClass,
        vec![],
    )
    .expect("spawn isolate");
    let result = isolate
        .probe(
            r#"(() => {
                const first = crypto.randomUUID();
                const second = crypto.randomUUID();
                const bytes = new Uint8Array(64);
                const sameArray = crypto.getRandomValues(bytes) === bytes;
                let quotaName = null;
                let quotaDomException = false;
                try {
                    crypto.getRandomValues(new Uint8Array(65537));
                } catch (error) {
                    quotaName = error.name;
                    quotaDomException = error instanceof DOMException.DOMException;
                }
                let typeMismatchName = null;
                let typeMismatchDomException = false;
                try {
                    crypto.getRandomValues(new Float32Array(1));
                } catch (error) {
                    typeMismatchName = error.name;
                    typeMismatchDomException = error instanceof DOMException.DOMException;
                }
                return {
                    firstValid: /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(first),
                    distinct: first !== second,
                    filled: bytes.some((byte) => byte !== 0),
                    sameArray,
                    quotaName,
                    quotaDomException,
                    typeMismatchName,
                    typeMismatchDomException,
                    subtleUndefined: crypto.subtle === undefined,
                    cryptoKeys: Object.getOwnPropertyNames(crypto).sort(),
                    urlUndefined: typeof URL === "undefined",
                    urlSearchParamsUndefined: typeof URLSearchParams === "undefined",
                };
            })()"#,
        )
        .expect("evaluate crypto stub assertions");

    assert_eq!(result["firstValid"], true);
    assert_eq!(result["distinct"], true);
    assert_eq!(result["filled"], true);
    assert_eq!(result["sameArray"], true);
    assert_eq!(result["quotaName"], "QuotaExceededError");
    assert_eq!(result["quotaDomException"], true);
    assert_eq!(result["typeMismatchName"], "TypeMismatchError");
    assert_eq!(result["typeMismatchDomException"], true);
    assert_eq!(result["subtleUndefined"], true);
    assert_eq!(
        result["cryptoKeys"],
        serde_json::json!(["getRandomValues", "randomUUID"])
    );
    assert_eq!(result["urlUndefined"], true);
    assert_eq!(result["urlSearchParamsUndefined"], true);
}
