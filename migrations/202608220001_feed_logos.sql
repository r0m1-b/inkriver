ALTER TABLE feeds ADD COLUMN site_url TEXT;
ALTER TABLE feeds ADD COLUMN declared_icon_url TEXT;
ALTER TABLE feeds ADD COLUMN logo_png BLOB;
ALTER TABLE feeds ADD COLUMN logo_site_url TEXT;
ALTER TABLE feeds ADD COLUMN logo_attempted_at TEXT;
ALTER TABLE feeds ADD COLUMN logo_attempted_site_url TEXT;
ALTER TABLE feeds ADD COLUMN logo_attempted_declared_url TEXT;
ALTER TABLE feeds ADD COLUMN logo_last_error TEXT;
