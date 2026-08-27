use api::query::Query;

#[test]
fn query_filters_chain_and_terminal() {
    let v = [1, 2, 3, 4, 5];
    let mut q = Query::new(&v);
    q.where_(|n| *n > 2).where_(|n| *n < 5);
    assert_eq!(q.results(), vec![&3, &4]);
    assert_eq!(q.first(), Some(&3));
    assert_eq!(q.last(), Some(&4));
    assert!(q.exists() && !q.empty());
    assert_eq!(q.count(), 2);
}
