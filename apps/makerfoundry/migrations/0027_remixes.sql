-- MF-47: remixes: a 3D model names the models it was made from. A parent's row stays when it's
-- taken down (it's named in the tree, without a link); a deleted model's links go with it.
CREATE TABLE mf_remixes (
    child_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    parent_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (child_id, parent_id),
    CHECK (child_id <> parent_id)
);
CREATE INDEX mf_remixes_parent ON mf_remixes (parent_id);
