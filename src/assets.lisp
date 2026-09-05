(in-package #:ekko/platform)

;; The daemon owns immutable snapshots. Store entries and in-flight scenes each
;; own one reference; presentation acknowledgements release the latter.
(defstruct local-asset path size (references 1))
(defvar *asset-directory* nil)
(defvar *asset-sequence* 0)
(defvar *asset-bytes* 0)
(defconstant +asset-limit+ (* 256 1024 1024))

(defun asset-size (data)
  (if (local-asset-p data) (local-asset-size data) (length data)))
(defun retain-asset (data)
  (when (local-asset-p data) (incf (local-asset-references data))) data)
(defun release-asset (data)
  (when (and (local-asset-p data) (zerop (decf (local-asset-references data))))
    (delete-file (local-asset-path data))
    (decf *asset-bytes* (local-asset-size data))))
(defun snapshot-asset (name size)
  (unless (and *asset-directory* (<= (+ *asset-bytes* size) +asset-limit+)
               (<= 2 (length name) 255) (char= (char name 0) #\/)
               (not (find #\/ name :start 1)) (not (find #\Null name)))
    (error "Invalid shared memory transfer or snapshot quota exceeded"))
  (let ((path (format nil "~Aframe-~D" *asset-directory* (incf *asset-sequence*))))
    (checked (snapshot-shm name path size) "shared memory snapshot")
    (incf *asset-bytes* size)
    (make-local-asset :path path :size size)))

(defun initialize-assets (directory)
  ;; Called only while holding this session's exclusive lock. Reclaim files
  ;; left by a daemon crash before accepting producers or attachments.
  (ensure-directories-exist directory)
  (let ((st (sb-posix:lstat (string-right-trim "/" directory))))
    (unless (and (sb-posix:s-isdir (sb-posix:stat-mode st))
                 (= (sb-posix:stat-uid st) (sb-posix:getuid)))
      (error "Unsafe asset directory")))
  (sb-posix:chmod directory #o700)
  (dolist (file (directory (concatenate 'string directory "frame-*"))) (delete-file file))
  (setf *asset-directory* directory *asset-sequence* 0 *asset-bytes* 0))

(export '(local-asset local-asset-p local-asset-path local-asset-size make-local-asset
          asset-size retain-asset release-asset snapshot-asset initialize-assets))
