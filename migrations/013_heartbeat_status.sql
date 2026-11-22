-- Seam warte-plugin/heartbeat-status (REQ-GLANCE-001, warte#179): the optional
-- status a heartbeat carries, ended, idle or working; NULL for a bodyless beat.
ALTER TABLE agents ADD COLUMN heartbeat_status TEXT;
