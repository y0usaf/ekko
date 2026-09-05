(in-package #:cl-user)

(defun run-graphics-parser-tests ()
  ;; The bulk path must behave identically for every read boundary, including
  ;; ESC/ST split across reads and control strings exactly at the quota.
  (dolist (size '(0 1 16384 16385))
    (dolist (chunk-size '(1 2 17 4096 65536))
      (let* ((vt (ekko/vt:make-terminal))
             (payload (make-array size :element-type '(unsigned-byte 8) :initial-element 65))
             (wire (concatenate '(vector (unsigned-byte 8)) #(27 95) payload #(27 92 90)))
             (received nil))
        (loop for start from 0 below (length wire) by chunk-size do
          (ekko/vt:feed vt (subseq wire start (min (length wire) (+ start chunk-size)))
                        (lambda (kind value) (when (eq kind :graphics) (push value received)))))
        (ekko-test "graphics parser quota/fragmentation"
                   (if (> size 16384) (null received)
                       (and (= (length received) 1) (equalp payload (first received)))))
        (ekko-test "graphics parser resumes text after ST"
                   (string= "Z" (first (aref (ekko/vt:terminal-cells vt) 0)))))))
  (let ((vt (ekko/vt:make-terminal)) (received nil))
    (ekko/vt:feed vt (coerce '(27 95 65 27 88 66 27 92 90) '(vector (unsigned-byte 8)))
                  (lambda (kind value) (declare (ignore value)) (push kind received)))
    (ekko-test "invalid graphics escape discarded" (null received))
    (ekko-test "invalid graphics escape recovery"
               (string= "Z" (first (aref (ekko/vt:terminal-cells vt) 0)))))
  ;; The final decoded chunk is shorter. Assembly must preserve chunk order
  ;; and use the previous chunk's size when advancing the write offset.
  (let* ((vt (ekko/vt:make-terminal)) (store (ekko/graphics:make-store))
         (pixels (make-array 20 :element-type '(unsigned-byte 8)
                                :initial-contents (loop for i below 20 collect i))))
    (loop for start in '(0 3 15) for end in '(3 15 20) do
      (ekko/graphics:accept-command store
        (ekko/platform:text-bytes
          (format nil "G~Am=~D;~A"
                  (if (zerop start) "a=T,f=32,s=5,v=1,i=7,C=1,q=2," "")
                  (if (= end 20) 0 1)
                  (ekko/graphics:base64-encode (subseq pixels start end))))
        vt (lambda (&rest args) (declare (ignore args)))))
    (let ((image (gethash 7 (ekko/graphics:store-images store))))
      (ekko-test "unequal upload chunks preserve every pixel"
        (and image (= 1 (ekko/graphics:store-frames store))
             (zerop (ekko/graphics:store-errors store))
             (equalp pixels (ekko/platform:decompress-bytes (ekko/graphics:image-data image) 20))))))
  t)
