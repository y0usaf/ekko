(defpackage #:ekko/runtime-tests
  (:use #:cl))
(in-package #:ekko/runtime-tests)

(defun check (ok message)
  (unless ok (error "input: ~A" message)))

(defun bytes (&rest values)
  (make-array (length values) :element-type '(unsigned-byte 8)
              :initial-contents values))

(defun packet-payloads (wire)
  (mapcar (lambda (packet) (subseq packet 5)) (ekko/runtime::wire-queue wire)))

(defun packet-types (wire)
  (mapcar (lambda (packet) (aref packet 4)) (ekko/runtime::wire-queue wire)))

(defun make-input-viewer ()
  (let ((wire (ekko/runtime::make-wire :fd -1)))
    (values (ekko/runtime::make-viewer :connection wire) wire)))

(defun run-input-tests ()
  ;; Coalescing must stop at the earliest protocol delimiter, and complete
  ;; escape sequences must remain one event.
  (multiple-value-bind (viewer wire) (make-input-viewer)
    (ekko/runtime::input-feed viewer (bytes 65 2 66 27 91 65 67) 7)
    (check (equal (packet-types wire) '(2 2 2 2 2)) "input event types")
    (check (equalp (packet-payloads wire)
                  (list (bytes 65) (bytes 2) (bytes 66)
                        (bytes 27 91 65) (bytes 67)))
           "earliest delimiter and escape boundary"))
  ;; Ordinary command bytes remain batched; the daemon interprets the prefix.
  (multiple-value-bind (viewer wire) (make-input-viewer)
    (ekko/runtime::input-feed viewer (bytes 2 82 73 71 72) 5)
    (check (equalp (packet-payloads wire)
                  (list (bytes 2) (bytes 82 73 71 72)))
           "prefix command batch"))
  ;; A read boundary between the prefix and its command must have the same
  ;; result as a coalesced read.
  (multiple-value-bind (viewer wire) (make-input-viewer)
    (ekko/runtime::input-feed viewer (bytes 2) 1)
    (ekko/runtime::input-feed viewer (bytes 82 73 71 72) 4)
    (check (equalp (packet-payloads wire)
                   (list (bytes 2) (bytes 82 73 71 72)))
           "fragmented prefix command"))
  ;; Kitty's Ctrl-b encoding must enter the same prefix state.
  (multiple-value-bind (viewer wire) (make-input-viewer)
    (ekko/runtime::input-feed viewer (bytes 27 91 57 56 59 53 117 122) 8)
    (check (equalp (packet-payloads wire)
                   (list (bytes 27 91 57 56 59 53 117) (bytes 122)))
           "kitty prefix command"))
  ;; Test dispatch, not just framing: changing focus must not swallow the
  ;; trailing bytes, whether Ctrl-b was raw, fragmented, or Kitty-encoded.
  (dolist (parts (list (list (bytes 2 50 82 73 71 72 84))
                      (list (bytes 2) (bytes 50 82 73 71 72 84))
                      (list (bytes 27 91 57 56 59 53 117 50 82 73 71 72 84))))
    (multiple-value-bind (viewer wire) (make-input-viewer)
      (let* ((left (ekko/runtime::make-pane :id 1 :vt (ekko/vt:make-terminal)
                                           :io (ekko/runtime::make-wire :fd -1)))
             (right (ekko/runtime::make-pane :id 2 :vt (ekko/vt:make-terminal)
                                            :io (ekko/runtime::make-wire :fd -1)))
             (session (ekko/runtime::make-session :panes (list left right))))
        (dolist (part parts) (ekko/runtime::input-feed viewer part (length part)))
        (dolist (payload (packet-payloads wire)) (ekko/runtime::input-key session payload))
        (check (= (ekko/runtime::session-focus session) 1) "batched focus command")
        (check (null (ekko/runtime::wire-queue (ekko/runtime::pane-io left))) "input left old pane")
        (check (equalp (ekko/runtime::wire-queue (ekko/runtime::pane-io right))
                       (list (bytes 82 73 71 72 84))) "input follows focus change"))))
  ;; A drained queue must release its tail so the next append starts a fresh
  ;; list and does not splice into the retired one.
  (ekko/platform:initialize)
  (let* ((fd (sb-posix:open "/dev/null" #o1))
         (wire (ekko/runtime::make-wire :fd fd)))
    (unwind-protect
         (progn
           (ekko/runtime::queue-bytes wire (bytes 1))
           (ekko/runtime::queue-bytes wire (bytes 2))
           (ekko/runtime::flush-wire wire)
           (check (and (null (ekko/runtime::wire-queue wire))
                       (null (ekko/runtime::wire-queue-tail wire)))
                  "queue drain clears tail")
           (ekko/runtime::queue-bytes wire (bytes 3))
           (check (eq (ekko/runtime::wire-queue wire)
                      (ekko/runtime::wire-queue-tail wire))
                  "queue reuse tail"))
      (ekko/runtime::close-wire wire)))
  ;; Bracketed paste starts and ends change packet routing without splitting
  ;; the ordinary payload into per-byte packets.
  (multiple-value-bind (viewer wire) (make-input-viewer)
    (ekko/runtime::input-feed viewer (bytes 27 91 50 48 48 126 65 66 27 91 50 48 49 126) 14)
    (check (equal (packet-types wire) '(5 5 5)) "paste packet types")
    (check (equalp (packet-payloads wire)
                  (list (bytes 27 91 50 48 48 126)
                        (bytes 65 66)
                        (bytes 27 91 50 48 49 126)))
           "paste boundaries"))
  (format t "input batching tests passed~%")
  t)
