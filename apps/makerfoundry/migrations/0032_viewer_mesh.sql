-- MF-46: the mesh the 3D viewer draws, made by the server at upload (src/model3d/viewer.rs).
ALTER TABLE mf_model_files ADD COLUMN mesh_key TEXT;
