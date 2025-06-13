use facet::Facet;

fn main() {
    #[derive(Facet)]
    struct Foo {
        foo: String,
    }

    #[cfg(feature = "facet-json")]
    {
        use facet_pretty::FacetPretty;
        let input = r#"{ "foo": "bar" }"#;
        let foo: Foo = facet_json::from_str(input).unwrap();
        println!("{}", foo.pretty());
    }

    #[cfg(feature = "facet-toml")]
    {
        use facet_pretty::FacetPretty;
        let input = r#"
foo = "bar"
"#;
        let foo: Foo = facet_toml::from_str(input).unwrap();
        println!("{}", foo.pretty());
    }

    println!("Done!");
}
