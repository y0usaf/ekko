(defpackage #:ekko/client
  (:use #:cl)
  (:export #:make-attachment #:attachment-teardown
           #:allocate-outer-id #:enqueue-transaction #:flush-transactions
           #:queued-bytes #:queued-transactions
           #:expect-outer-reply #:accept-outer-reply #:expire-outer-replies))

(in-package #:ekko/client)

(defstruct (attachment (:constructor %make-attachment))
  (next-id 1 :type integer)
  (max-id 4294967295 :type (unsigned-byte 32))
  (ids (make-hash-table :test #'equal))
  (replies (make-hash-table))
  (queue '())
  (bytes 0 :type fixnum)
  (transactions 0 :type fixnum)
  (max-bytes (* 1024 1024) :type fixnum)
  (max-transactions 256 :type fixnum)
  (max-mappings 65536 :type fixnum)
  (closed-p nil))

(defun make-attachment (&key (max-bytes (* 1024 1024)) (max-transactions 256)
                              (max-id 4294967295) (max-mappings 65536))
  (check-type max-bytes (integer 1 #.most-positive-fixnum))
  (check-type max-transactions (integer 1 #.most-positive-fixnum))
  (check-type max-mappings (integer 1 #.most-positive-fixnum))
  (check-type max-id (integer 1 4294967295))
  (%make-attachment :max-bytes max-bytes :max-transactions max-transactions
                    :max-id max-id :max-mappings max-mappings))

(defun live (attachment)
  (when (attachment-closed-p attachment) (error "Attachment is closed"))
  attachment)

(defun uint32 (value name)
  (unless (typep value '(integer 0 4294967295))
    (error "~A must be an unsigned 32-bit integer: ~S" name value))
  value)

(defun allocate-outer-id (attachment pane-id incarnation child-image-id generation
                          &optional placement-id)
  "Return a stable outer ID for one attachment-owned image generation." 
  (live attachment)
  (uint32 pane-id 'pane-id)
  (uint32 incarnation 'incarnation)
  (uint32 child-image-id 'child-image-id)
  (uint32 generation 'generation)
  (when placement-id (uint32 placement-id 'placement-id))
  (let* ((key (list pane-id incarnation child-image-id generation placement-id))
         (old (gethash key (attachment-ids attachment))))
    (or old
        (let ((id (attachment-next-id attachment)))
          (when (>= (hash-table-count (attachment-ids attachment))
                    (attachment-max-mappings attachment))
            (error "Attachment mapping limit exceeded"))
          (when (> id (attachment-max-id attachment)) (error "Outer image ID space exhausted"))
          (setf (gethash key (attachment-ids attachment)) id
                (attachment-next-id attachment) (1+ id))
          id))))

(defstruct outer-request kind placement deadline (state :pending))

(defun expect-outer-reply (attachment outer-id kind placement-id deadline)
  "Track one decoded host response for an allocated ID. IDs are single-use for
requests until detach, so a duplicate or late reply cannot satisfy a new request.
DEADLINE and the receiving clock use the executor's monotonic time units."
  (live attachment)
  (uint32 outer-id 'outer-id)
  (uint32 placement-id 'placement-id)
  (unless (member kind '(:query :upload :placement)) (error "Invalid request kind"))
  (check-type deadline (integer 0 *))
  (unless (< 0 outer-id (attachment-next-id attachment))
    (error "Reply identity was not allocated by this attachment"))
  (when (gethash outer-id (attachment-replies attachment))
    (error "Reply identity already used; allocate a new generation"))
  (setf (gethash outer-id (attachment-replies attachment))
        (make-outer-request :kind kind :placement placement-id :deadline deadline))
  outer-id)

(defun expire-outer-replies (attachment now)
  "Expire pending requests, retaining tombstones bounded by the mapping quota."
  (live attachment)
  (check-type now (integer 0 *))
  (let ((expired 0))
    (maphash (lambda (id request)
               (declare (ignore id))
               (when (and (eq :pending (outer-request-state request))
                          (>= now (outer-request-deadline request)))
                 (setf (outer-request-state request) :expired)
                 (incf expired)))
             (attachment-replies attachment))
    expired))

(defun accept-outer-reply (attachment outer-id placement-id now success-p)
  "Consume a validated, decoded host reply into presentation health only.
Return health and request kind, never a pane or application reply. The executor
must select ATTACHMENT from the input transport, never from current pane focus."
  (live attachment)
  (uint32 outer-id 'outer-id)
  (uint32 placement-id 'placement-id)
  (check-type now (integer 0 *))
  (unless (member success-p '(nil t)) (error "Success must be a boolean"))
  (let ((request (gethash outer-id (attachment-replies attachment))))
    (cond
      ((null request) :unknown)
      ((not (eq :pending (outer-request-state request))) :stale)
      ((>= now (outer-request-deadline request))
       (setf (outer-request-state request) :expired)
       :stale)
      ((/= placement-id (outer-request-placement request)) :mismatch)
      (t
       (setf (outer-request-state request) (if success-p :accepted :failed))
       (values (outer-request-state request) (outer-request-kind request))))))

(defun octets-copy (octets)
  (unless (typep octets '(vector (unsigned-byte 8)))
    (error "Transaction must be an octet vector"))
  (copy-seq octets))

(defun enqueue-transaction (attachment octets)
  "Copy and append one complete transaction, enforcing caps before mutation." 
  (live attachment)
  (unless (typep octets '(vector (unsigned-byte 8)))
    (error "Transaction must be an octet vector"))
  (unless (plusp (length octets)) (error "Empty transaction"))
  (let ((size (length octets)))
    (when (> (+ (attachment-bytes attachment) size) (attachment-max-bytes attachment))
      (error "Transaction byte queue limit exceeded"))
    (when (>= (attachment-transactions attachment) (attachment-max-transactions attachment))
      (error "Transaction count queue limit exceeded"))
    (let ((copy (octets-copy octets)))
      (setf (attachment-queue attachment)
            (nconc (attachment-queue attachment) (list (cons copy 0)))
            (attachment-bytes attachment) (+ (attachment-bytes attachment) size)
            (attachment-transactions attachment) (1+ (attachment-transactions attachment))))
    size))

(defun flush-transactions (attachment writer &key (max-bytes 65536)
                                                   (max-calls 64))
  "Write queued octets in order. WRITER receives (vector start count), and may
return a short positive count, zero, or :EAGAIN. Zero/EAGAIN preserve the
queue. Invalid counts signal an error. Budget exhaustion yields :WOULD-BLOCK.
WRITER is a trusted, non-reentrant host executor; it must not modify the vector." 
  (live attachment)
  (check-type max-bytes (integer 1 *))
  (check-type max-calls (integer 1 *))
  (let ((written 0) (calls 0))
   (loop while (and (attachment-queue attachment) (< written max-bytes) (< calls max-calls))
        for item = (first (attachment-queue attachment))
        for vector = (car item)
        for offset = (cdr item)
        for count = (- (length vector) offset)
        for allowed = (min count (- max-bytes written))
        for result = (progn (incf calls) (funcall writer vector offset allowed))
        do (cond
             ((or (eq result :eagain) (and (integerp result) (zerop result))) (return-from flush-transactions :would-block))
             ((and (integerp result) (plusp result) (<= result allowed))
              (incf (cdr item) result)
              (incf written result)
              (when (= (cdr item) (length vector))
                (pop (attachment-queue attachment))
                (decf (attachment-bytes attachment) (length vector))
                (decf (attachment-transactions attachment))))
             (t (error "Writer returned invalid progress ~S for ~D bytes" result allowed))))
   (if (attachment-queue attachment) :would-block :drained)))

(defun attachment-teardown (attachment)
  (unless (attachment-closed-p attachment)
    (setf (attachment-queue attachment) nil
          (attachment-bytes attachment) 0
          (attachment-transactions attachment) 0
          (attachment-closed-p attachment) t)
    (clrhash (attachment-ids attachment))
    (clrhash (attachment-replies attachment)))
  t)

(defun queued-bytes (attachment) (attachment-bytes attachment))
(defun queued-transactions (attachment) (attachment-transactions attachment))
