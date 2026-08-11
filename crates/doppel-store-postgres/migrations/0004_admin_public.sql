-- `AdminConfig::public`: serve the admin API unauthenticated.
--
-- Nullable, with no DDL default, for the reason given in 0003: the field is an
-- `Option<bool>` whose absence is a different document from `false` written out,
-- and the revision is derived from the document's canonical YAML. A column that
-- materialised `false` would make every configuration stored before this
-- migration fail its own revision check on the first load after it.
ALTER TABLE configurations ADD COLUMN admin_public BOOLEAN;
