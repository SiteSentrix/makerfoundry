-- MF-55: consent to feature a creator's project on MakerFoundry's social pages, given by ticking an
-- unticked box (on a 3D upload, a make of a 3D model, or an import's publishing), kept with its
-- time, and withdrawn at any time (or when the account's data is deleted). A withdrawn consent
-- keeps its times, so posts made from it can be found and taken down (MF-56).
CREATE TABLE mf_showcase_consents (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    design_id BIGINT REFERENCES mf_designs (id) ON DELETE SET NULL,
    make_id BIGINT REFERENCES mf_makes (id) ON DELETE SET NULL,
    given_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    withdrawn_at TIMESTAMPTZ
);
CREATE INDEX mf_showcase_consents_account ON mf_showcase_consents (account_id, given_at DESC);
CREATE INDEX mf_showcase_consents_active ON mf_showcase_consents (given_at DESC) WHERE withdrawn_at IS NULL;
