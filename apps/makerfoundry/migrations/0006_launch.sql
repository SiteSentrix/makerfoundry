-- MF-6: who agreed to which legal document, and which version of it (the start of its text's
-- SHA-256): the Terms and the Privacy Policy when signing up, the Seller Terms when publishing or
-- setting up selling.
CREATE TABLE mf_agreements (
    account_id BIGINT NOT NULL,
    document TEXT NOT NULL CHECK (document IN ('terms', 'privacy', 'seller-terms')),
    version TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, document, version)
);
