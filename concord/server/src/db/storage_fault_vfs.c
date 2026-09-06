#include <sqlite3.h>
#include <stdatomic.h>
#include <stddef.h>
#include <string.h>

typedef struct ConcordFaultFile {
  sqlite3_file base;
  sqlite3_file *real;
  const sqlite3_io_methods *real_methods;
  int fault_target;
} ConcordFaultFile;

static sqlite3_vfs *underlying;
static sqlite3_vfs fault_vfs;
static sqlite3_io_methods fault_io;
static atomic_int fail_next_sync;
static atomic_int observed_sync_faults;
static size_t real_file_offset;

#define FILE_WRAPPER(file) ((ConcordFaultFile *)(file))
#define DELEGATE0(wrapper, member)                                             \
  static int wrapper(sqlite3_file *file) {                                     \
    return FILE_WRAPPER(file)->real_methods->member(FILE_WRAPPER(file)->real); \
  }
#define DELEGATE1(wrapper, member, type1, arg1)                                \
  static int wrapper(sqlite3_file *file, type1 arg1) {                         \
    return FILE_WRAPPER(file)->real_methods->member(FILE_WRAPPER(file)->real,  \
                                                     arg1);                     \
  }
#define DELEGATE2(wrapper, member, type1, arg1, type2, arg2)                   \
  static int wrapper(sqlite3_file *file, type1 arg1, type2 arg2) {             \
    return FILE_WRAPPER(file)->real_methods->member(FILE_WRAPPER(file)->real,  \
                                                     arg1, arg2);               \
  }
#define DELEGATE3(wrapper, member, type1, arg1, type2, arg2, type3, arg3)      \
  static int wrapper(sqlite3_file *file, type1 arg1, type2 arg2, type3 arg3) { \
    return FILE_WRAPPER(file)->real_methods->member(                           \
        FILE_WRAPPER(file)->real, arg1, arg2, arg3);                            \
  }

static int fault_close(sqlite3_file *file) {
  int result = FILE_WRAPPER(file)->real_methods->xClose(FILE_WRAPPER(file)->real);
  FILE_WRAPPER(file)->base.pMethods = NULL;
  return result;
}

DELEGATE3(fault_read, xRead, void *, buffer, int, amount, sqlite3_int64, offset)
DELEGATE3(fault_write, xWrite, const void *, buffer, int, amount, sqlite3_int64,
          offset)
DELEGATE1(fault_truncate, xTruncate, sqlite3_int64, size)

static int fault_sync(sqlite3_file *file, int flags) {
  int armed = 1;
  if (FILE_WRAPPER(file)->fault_target &&
      atomic_compare_exchange_strong(&fail_next_sync, &armed, 0)) {
    atomic_fetch_add(&observed_sync_faults, 1);
    return SQLITE_IOERR_FSYNC;
  }
  return FILE_WRAPPER(file)->real_methods->xSync(FILE_WRAPPER(file)->real,
                                                 flags);
}

DELEGATE1(fault_file_size, xFileSize, sqlite3_int64 *, size)
DELEGATE1(fault_lock, xLock, int, lock)
DELEGATE1(fault_unlock, xUnlock, int, lock)
DELEGATE1(fault_check_reserved_lock, xCheckReservedLock, int *, result)
DELEGATE2(fault_file_control, xFileControl, int, operation, void *, argument)
DELEGATE0(fault_sector_size, xSectorSize)
DELEGATE0(fault_device_characteristics, xDeviceCharacteristics)

static int fault_shm_map(sqlite3_file *file, int page, int page_size, int extend,
                         void volatile **mapped) {
  return FILE_WRAPPER(file)->real_methods->xShmMap(
      FILE_WRAPPER(file)->real, page, page_size, extend, mapped);
}

static int fault_shm_lock(sqlite3_file *file, int offset, int count, int flags) {
  return FILE_WRAPPER(file)->real_methods->xShmLock(FILE_WRAPPER(file)->real,
                                                    offset, count, flags);
}

static void fault_shm_barrier(sqlite3_file *file) {
  FILE_WRAPPER(file)->real_methods->xShmBarrier(FILE_WRAPPER(file)->real);
}

static int fault_shm_unmap(sqlite3_file *file, int delete_flag) {
  return FILE_WRAPPER(file)->real_methods->xShmUnmap(FILE_WRAPPER(file)->real,
                                                     delete_flag);
}

static int fault_fetch(sqlite3_file *file, sqlite3_int64 offset, int amount,
                       void **value) {
  if (FILE_WRAPPER(file)->real_methods->iVersion < 3 ||
      FILE_WRAPPER(file)->real_methods->xFetch == NULL) {
    *value = NULL;
    return SQLITE_OK;
  }
  return FILE_WRAPPER(file)->real_methods->xFetch(FILE_WRAPPER(file)->real,
                                                  offset, amount, value);
}

static int fault_unfetch(sqlite3_file *file, sqlite3_int64 offset, void *value) {
  if (FILE_WRAPPER(file)->real_methods->iVersion < 3 ||
      FILE_WRAPPER(file)->real_methods->xUnfetch == NULL) {
    return SQLITE_OK;
  }
  return FILE_WRAPPER(file)->real_methods->xUnfetch(FILE_WRAPPER(file)->real,
                                                    offset, value);
}

static int fault_open(sqlite3_vfs *vfs, const char *name, sqlite3_file *file,
                      int flags, int *out_flags) {
  (void)vfs;
  ConcordFaultFile *wrapper = FILE_WRAPPER(file);
  memset(wrapper, 0, sizeof(*wrapper));
  wrapper->real = (sqlite3_file *)((unsigned char *)file + real_file_offset);
  int result =
      underlying->xOpen(underlying, name, wrapper->real, flags, out_flags);
  if (result != SQLITE_OK) {
    return result;
  }
  wrapper->real_methods = wrapper->real->pMethods;
  wrapper->fault_target = name != NULL && strstr(name, "sync-fault.db") != NULL;
  wrapper->base.pMethods = &fault_io;
  return SQLITE_OK;
}

int concord_storage_fault_vfs_install(void) {
  if (underlying != NULL) {
    return SQLITE_MISUSE;
  }
  underlying = sqlite3_vfs_find(NULL);
  if (underlying == NULL) {
    return SQLITE_NOTFOUND;
  }
  const size_t alignment = _Alignof(max_align_t);
  real_file_offset =
      (sizeof(ConcordFaultFile) + alignment - 1U) & ~(alignment - 1U);
  fault_vfs = *underlying;
  fault_vfs.zName = "concord-storage-fault";
  fault_vfs.szOsFile = (int)(real_file_offset + (size_t)underlying->szOsFile);
  fault_vfs.xOpen = fault_open;
  fault_io = (sqlite3_io_methods){
      3,
      fault_close,
      fault_read,
      fault_write,
      fault_truncate,
      fault_sync,
      fault_file_size,
      fault_lock,
      fault_unlock,
      fault_check_reserved_lock,
      fault_file_control,
      fault_sector_size,
      fault_device_characteristics,
      fault_shm_map,
      fault_shm_lock,
      fault_shm_barrier,
      fault_shm_unmap,
      fault_fetch,
      fault_unfetch,
  };
  return sqlite3_vfs_register(&fault_vfs, 1);
}

void concord_storage_fault_vfs_arm_next_sync(void) {
  atomic_store(&fail_next_sync, 1);
}

int concord_storage_fault_vfs_observed(void) {
  return atomic_load(&observed_sync_faults);
}
