#!/bin/bash -e
source /pi-gen/test-tools/test-cache.sh
test_cache_restore_pristine "$WORK_DIR/stage2/rootfs" "$ROOTFS_DIR" "$WORK_DIR"
