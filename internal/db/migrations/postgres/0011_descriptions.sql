-- Optional free-text description for a project and a check, rendered with the
-- minimal markdown subset in `src/markdown.rs`. NOT NULL DEFAULT '' so every
-- existing row gets "no description" without the model needing an Option.
ALTER TABLE projects ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE checks ADD COLUMN description TEXT NOT NULL DEFAULT '';
