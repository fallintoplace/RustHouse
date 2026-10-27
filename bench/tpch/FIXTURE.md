# Deterministic filtered-revenue fixture

This fixture is synthetic and only resembles a small part of TPC-H.
It is not official TPC-H dbgen data and cannot produce certified scores.
Prices are stored in minor units. Discounts are integer basis points.
The expected revenue is in price-minor-units times discount basis points.

Generate a 60,000-row fixture:

    python3 bench/tpch/make_fixture.py --rows 60000 --seed 42 --output-dir /tmp/rusthouse-q6

Create and load a local database with RustHouse:

    target/release/rusthouse sql --file /tmp/rusthouse-q6/create.sql --database /tmp/q6db
    target/release/rusthouse sql --file /tmp/rusthouse-q6/copy.sql --database /tmp/q6db

Then run the query file and compare against references/q06.tsv.
Generation and import are outside the timed query measurements.

Run generator tests:

    python3 -m unittest discover -s bench/tpch -p 'test_make_fixture.py'
