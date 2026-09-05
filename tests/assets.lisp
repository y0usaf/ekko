(in-package #:cl-user)

(defun run-asset-tests ()
  (ekko/platform:initialize)
  (let* ((directory (format nil "~Aekko-assets-~D/" (uiop:temporary-directory) (sb-posix:getpid)))
         (name (format nil "/ekko-assets-test-~D" (sb-posix:getpid)))
         (source (concatenate 'string "/dev/shm" name))
         (ekko/platform::*asset-directory* nil)
         (ekko/platform::*asset-bytes* 0)
         (ekko/platform::*asset-sequence* 0)
         (store (ekko/graphics:make-store)) (vt (ekko/vt:make-terminal))
         (replies nil) (wire (ekko/runtime::make-wire :fd -1)))
    (labels ((write-source (&optional (bytes #(255 0 0 255)))
               (with-open-file (out source :direction :output :if-exists :supersede
                                           :element-type '(unsigned-byte 8))
                 (write-sequence bytes out)))
             (send (header &optional (payload name))
               (ekko/graphics:accept-command store
                 (ekko/platform:text-bytes
                   (format nil "G~A;~A" header (ekko/graphics:base64-encode (ekko/platform:text-bytes payload))))
                 vt (lambda (kind value) (when (eq kind :reply) (push value replies)))))
             (image () (gethash 7 (ekko/graphics:store-images store))))
      (unwind-protect
           (progn
             (ekko/platform:initialize-assets directory)
             (write-source)
             (send "a=q,t=s,f=32,s=1,v=1,i=299")
             (ekko-test "shared query consumes object without retaining snapshot"
                        (and (search "OK" (first replies)) (not (probe-file source))
                             (zerop ekko/platform::*asset-bytes*) (null (image))))
             (write-source)
             (send "a=T,t=s,f=32,s=1,v=1,i=7,C=1")
             (let* ((data (ekko/graphics:image-data (image)))
                    (path (ekko/platform:local-asset-path data)))
               (ekko-test "raw snapshot charged once" (= 4 ekko/platform::*asset-bytes*))
               (setf (ekko/runtime::wire-leases wire) (list (ekko/platform:retain-asset data)))
               (write-source #(0 0 255 255))
               (send "a=T,t=s,f=32,s=1,v=1,i=7,C=1")
               (ekko-test "replacement preserves leased immutable pixels"
                 (and (probe-file path) (= 8 ekko/platform::*asset-bytes*)
                      (equalp #(255 0 0 255)
                        (ekko/platform:decompress-bytes (ekko/runtime::compressed-asset-data data) 4))))
               (ekko/runtime::acknowledge-scene wire)
               (ekko-test "ack releases replaced snapshot"
                          (and (not (probe-file path)) (= 4 ekko/platform::*asset-bytes*))))
             ;; Invalid uploads preserve the previous accepted generation.
             (let ((previous (image)) (errors (ekko/graphics:store-errors store)))
               (write-source #(1 2 3))
               (send "a=T,t=s,f=32,s=1,v=1,i=7,C=1")
               (ekko-test "short shared object rejected without unlink"
                          (and (eq previous (image)) (probe-file source)
                               (= (1+ errors) (ekko/graphics:store-errors store))))
               (write-source)
               (dolist (payload (list "/../etc/passwd" "relative" (format nil "~A~Cextra" name #\Null)))
                 (send "a=T,t=s,f=32,s=1,v=1,i=7,C=1" payload))
               (send "a=T,t=s,f=32,s=1,v=1,i=7,C=1,m=1")
               (send "a=T,t=s,f=32,s=1,v=1,i=7,C=1,o=z")
               (send "a=T,t=s,f=32,s=8192,v=8192,i=7,C=1")
               (let ((ekko/platform::*asset-bytes* ekko/platform::+asset-limit+))
                 (send "a=T,t=s,f=32,s=1,v=1,i=7,C=1"))
               (ekko-test "invalid paths, modes, sizes and quotas preserve image"
                          (and (eq previous (image))
                               (= (+ errors 8) (ekko/graphics:store-errors store)))))
             (let* ((data (ekko/graphics:image-data (image))) (path (ekko/platform:local-asset-path data)))
               (setf (ekko/runtime::wire-leases wire) (list (ekko/platform:retain-asset data)))
               (ekko/graphics:clear-screen store :main)
               (ekko-test "delete during presentation keeps leased pixels" (probe-file path))
               (ekko/runtime::close-wire wire)
               (ekko-test "client death releases final lease"
                          (and (not (probe-file path)) (zerop ekko/platform::*asset-bytes*)))))
        (ekko/graphics:clear-screen store :main)
        (ekko/runtime::close-wire wire)
        (when (probe-file source) (delete-file source))
        (uiop:delete-directory-tree (pathname directory) :validate t :if-does-not-exist :ignore))))
  ;; Fragmentation and unrelated host replies cannot release another upload or
  ;; escape into child input. A probe failure selects the inline fallback.
  (let* ((wire (ekko/runtime::make-wire :fd -1))
         (viewer (ekko/runtime::make-viewer :connection wire :transport :probing)))
    (setf (gethash 12 (ekko/runtime::viewer-uploads viewer)) 0)
    (dolist (reply '("i=13,p=1;OK" "i=12,p=2;OK" "i=4294967295;ENOTSUP"))
      (let ((bytes (ekko/platform:text-bytes (format nil "~C_G~A~C\\" #\Esc reply #\Esc))))
        (loop for byte across bytes do
          (ekko/runtime::input-feed viewer (make-array 1 :element-type '(unsigned-byte 8) :initial-element byte) 1))))
    (ekko-test "host replies stay owned by client"
               (and (null (ekko/runtime::wire-queue wire))
                    (gethash 12 (ekko/runtime::viewer-uploads viewer))
                    (eq :inline (ekko/runtime::viewer-transport viewer)))))
  t)
