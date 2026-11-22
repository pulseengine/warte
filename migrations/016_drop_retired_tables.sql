-- REQ-RETIRE-001 (#37, #38): the artifact store and multi-instance sync were
-- removed in v1.6.44. The operator confirmed all three tables held 0 rows in
-- production before this drop (2026-11-22).
DROP TABLE IF EXISTS artifacts;
DROP TABLE IF EXISTS sync_state;
DROP TABLE IF EXISTS instances;
