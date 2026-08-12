ALTER TABLE feeds ADD COLUMN title TEXT;
ALTER TABLE feeds ADD COLUMN description TEXT;
ALTER TABLE feeds ADD COLUMN author TEXT;
ALTER TABLE feeds ADD COLUMN last_success_at TEXT;
ALTER TABLE feeds ADD COLUMN last_error_stage TEXT;
ALTER TABLE feeds ADD COLUMN last_error_message TEXT;
ALTER TABLE feeds ADD COLUMN last_error_at TEXT;
