use orm::numeric::NumericText;
use tokio_postgres::NoTls;

#[test]
fn numeric_text_accepts_plain_decimals_only() {
    for value in ["0", "0.0000000000000000001", "123456789012345678901234567890", "-10.25"] {
        assert_eq!(NumericText::new(value).expect("valid decimal").as_str(), value);
    }
    assert_eq!(NumericText::new("-0.000").expect("negative zero").as_str(), "0.000");

    for value in ["", "-", ".1", "1.", "1e5", "1.2.3", "NaN", "Infinity"] {
        assert!(NumericText::new(value).is_err(), "accepted {value}");
    }

    let from_json: NumericText = serde_json::from_str("\"1.250\"").expect("deserialize decimal string");
    assert_eq!(from_json.as_str(), "1.250");
    assert!(serde_json::from_str::<NumericText>("\"1e5\"").is_err());
    assert_eq!(serde_json::to_string(&from_json).expect("serialize decimal string"), "\"1.250\"");
}

#[tokio::test]
async fn numeric_text_round_trips_through_postgres_without_floating_point() {
    let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
        eprintln!("skipping: set ORM_TEST_DATABASE_URL to run the numeric round-trip test");
        return;
    };

    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.expect("connect");
    let conn = tokio::spawn(async move {
        let _ = connection.await;
    });

    for value in [
        "0",
        "-0.0000",
        "0.0000000000000000001",
        "123456789012345678901234567890.12345678901234567890",
        "-987654321.0000000001",
        "10000.0000",
    ] {
        let parameter = NumericText::new(value).expect("valid decimal");
        let row = client
            .query_one("SELECT $1::numeric AS amount", &[&parameter])
            .await
            .expect("query numeric");
        let actual: NumericText = row.get("amount");

        assert_eq!(actual, parameter);
    }

    let values = vec![
        NumericText::new("0.0000000000000000001").expect("small decimal"),
        NumericText::new("12345678901234567890.1250").expect("large decimal"),
        NumericText::new("-9.75").expect("negative decimal"),
    ];
    let row = client
        .query_one("SELECT $1::numeric[] AS amounts", &[&values])
        .await
        .expect("query numeric array");
    let actual: Vec<NumericText> = row.get("amounts");
    assert_eq!(actual, values);

    for query in [
        "SELECT 'NaN'::numeric AS amount",
        "SELECT 'Infinity'::numeric AS amount",
        "SELECT '-Infinity'::numeric AS amount",
    ] {
        let row = client
            .query_one(query, &[])
            .await
            .expect("query special numeric");
        assert!(row.try_get::<_, NumericText>("amount").is_err());
    }

    conn.abort();
}
