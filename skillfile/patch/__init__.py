"""Patch generation, application, pin/unpin/diff/resolve."""

# Re-export patch.patch symbols for backward compat (from skillfile.patch import ...).
# Do NOT import pin/diff/resolve here — they create circular dependencies with deploy.
from .patch import PATCHES_DIR as PATCHES_DIR
from .patch import PatchConflictError as PatchConflictError
from .patch import apply_patch_pure as apply_patch_pure
from .patch import dir_patch_path as dir_patch_path
from .patch import generate_patch as generate_patch
from .patch import has_dir_patch as has_dir_patch
from .patch import has_patch as has_patch
from .patch import patch_path as patch_path
from .patch import patches_root as patches_root
from .patch import read_patch as read_patch
from .patch import remove_all_dir_patches as remove_all_dir_patches
from .patch import remove_dir_patch as remove_dir_patch
from .patch import remove_patch as remove_patch
from .patch import write_dir_patch as write_dir_patch
from .patch import write_patch as write_patch
