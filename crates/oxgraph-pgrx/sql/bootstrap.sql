CREATE SCHEMA IF NOT EXISTS graph;

CREATE TABLE IF NOT EXISTS graph._registered_tables (
    table_id integer PRIMARY KEY,
    schema_name text NOT NULL,
    table_name text NOT NULL,
    primary_key_column text NOT NULL
);

CREATE TABLE IF NOT EXISTS graph._registered_edges (
    edge_id integer PRIMARY KEY,
    source_table_id integer NOT NULL REFERENCES graph._registered_tables (table_id),
    target_table_id integer NOT NULL REFERENCES graph._registered_tables (table_id),
    source_column text NOT NULL,
    target_column text NOT NULL,
    schema_name text NOT NULL,
    table_name text NOT NULL
);

CREATE TABLE IF NOT EXISTS graph._registered_filter_columns (
    table_id integer NOT NULL REFERENCES graph._registered_tables (table_id),
    column_name text NOT NULL,
    PRIMARY KEY (table_id, column_name)
);

CREATE TABLE IF NOT EXISTS graph._sync_log (
    sequence bigint PRIMARY KEY,
    action_type smallint NOT NULL,
    arg0 bigint,
    arg1 bigint
);

CREATE TABLE IF NOT EXISTS graph._snapshot_store (
    id integer PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    bytes bytea NOT NULL DEFAULT ''::bytea,
    built_at_unix bigint NOT NULL DEFAULT 0
);

CREATE OR REPLACE FUNCTION graph._edge_change_sync_trigger() RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    next_seq bigint;
    src_pk bigint;
    dst_pk bigint;
    source_table_id integer;
    target_table_id integer;
    source_key bigint;
    target_key bigint;
BEGIN
    source_table_id := TG_ARGV[0]::integer;
    target_table_id := TG_ARGV[1]::integer;
    SELECT COALESCE(MAX(sequence), 0) + 1 INTO next_seq FROM graph._sync_log;
    IF TG_OP = 'INSERT' THEN
        EXECUTE format(
            'SELECT ($1).%I::bigint, ($1).%I::bigint',
            TG_ARGV[2],
            TG_ARGV[3]
        )
        INTO src_pk, dst_pk
        USING NEW;
        IF src_pk < 0 OR dst_pk < 0 OR src_pk > 4294967295 OR dst_pk > 4294967295 THEN
            RAISE EXCEPTION 'oxgraph sync primary key out of range';
        END IF;
        source_key := (source_table_id::bigint << 32) | src_pk;
        target_key := (target_table_id::bigint << 32) | dst_pk;
        INSERT INTO graph._sync_log (sequence, action_type, arg0, arg1)
        VALUES (next_seq, 1, source_key, target_key);
        RETURN NEW;
    ELSIF TG_OP = 'DELETE' THEN
        EXECUTE format(
            'SELECT ($1).%I::bigint, ($1).%I::bigint',
            TG_ARGV[2],
            TG_ARGV[3]
        )
        INTO src_pk, dst_pk
        USING OLD;
        IF src_pk < 0 OR dst_pk < 0 OR src_pk > 4294967295 OR dst_pk > 4294967295 THEN
            RAISE EXCEPTION 'oxgraph sync primary key out of range';
        END IF;
        source_key := (source_table_id::bigint << 32) | src_pk;
        target_key := (target_table_id::bigint << 32) | dst_pk;
        INSERT INTO graph._sync_log (sequence, action_type, arg0, arg1)
        VALUES (next_seq, 5, source_key, target_key);
        RETURN OLD;
    END IF;
    RETURN NULL;
END;
$$;
