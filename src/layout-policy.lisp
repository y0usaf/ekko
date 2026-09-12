(in-package #:ekko/runtime)

;; One state per view. The owner contribution is reconstructible; terminal
;; contents and PTYs are not owned by a mounted layout provider.
(defstruct layout-state owner provider placements (camera '(0 0))
  requested-context requested-token committed-context committed-token error)
(defstruct layout-request owner provider reads snapshot token)
(defconstant +layout-deadline+ 1/2)
(defconstant +layout-coordinate-limit+ 1000000)

(defun layout-provider (registry)
  (let ((name (getf (getf registry :options) :layout-provider)))
    (when name
      (or (find name (reverse (getf registry :layout-providers))
                :key (lambda (entry) (getf entry :name)) :test #'equal)
          (error "Selected layout provider is not registered: ~A" name)))))

(defun policy-plist (value keys required description)
  "Check a proper, unique-key property list once at the worker boundary."
  (unless (and (listp value) (evenp (length value)))
    (error "Malformed ~A" description))
  (let ((seen nil))
    (loop for (key item) on value by #'cddr do
      (unless (and (member key keys) (not (member key seen)))
        (error "Invalid or duplicate ~A key: ~S" description key))
      (push key seen))
    (unless (every (lambda (key) (member key seen)) required)
      (error "Missing ~A field" description)))
  value)

(defun validate-layout-registry (registry)
  (let* ((providers (getf registry :layout-providers))
         (options (getf registry :options))
         (budget (getf options :pane-budget 16)))
    (unless (and (listp providers) (<= (length providers) 64)
                 (typep budget '(integer 1 128))
                 (member (getf options :workspace-scope :session) '(:session :all-panes)))
      (error "Invalid layout registry or workspace budget"))
    (dolist (provider providers)
      (policy-plist provider '(:name :owner :api-version :reads)
                    '(:name :owner :api-version :reads) "layout registration")
      (let ((reads (getf provider :reads)))
        (unless (and (stringp (getf provider :name))
                     (eql (getf provider :api-version) 1)
                     (find (getf provider :owner) (getf registry :components)
                           :key (lambda (entry) (getf entry :id)) :test #'equal)
                     (listp reads)
                     (= (length reads) (length (remove-duplicates reads)))
                     (every (lambda (key) (member key ekko/extensions::*context-keys*)) reads)
                     (every (lambda (key) (member key reads)) '(:panes :focus :viewport :geometry)))
          (error "Invalid layout provider owner, version or dependencies"))))
    (let ((initial (getf options :initial-layout)))
      (when (and initial (> (length (ekko/extensions::initial-layout-leaves initial)) budget))
        (error "Initial layout exceeds the configured pane budget")))
    (layout-provider registry))
  t)

(defun reset-layout-state (state registry)
  (let* ((provider (layout-provider registry))
         (owner (getf provider :owner)) (name (getf provider :name)))
    (unless (and (equal owner (layout-state-owner state))
                 (equal name (layout-state-provider state)))
      (setf (layout-state-owner state) owner (layout-state-provider state) name
            (layout-state-placements state) nil (layout-state-camera state) '(0 0)
            (layout-state-requested-context state) nil (layout-state-requested-token state) nil
            (layout-state-committed-context state) nil (layout-state-committed-token state) nil
            (layout-state-error state) nil)
      t)))

(defun layout-pane-metadata (pane)
  ;; Do not feed committed projection, PTY arbitration, output counters or
  ;; history length back into the provider's own structural dependency.
  (loop for key in '(:id :session :name :label :argv :launch-kind :creation-position
                    :minimized :floating :activation-order :terminal-title :activity :pid :exit)
        when (member key pane) append (list key (ekko/extensions::copy-data (getf pane key)))))

(defun layout-policy-context (provider context)
  (loop for key in (getf provider :reads)
        for value = (getf context key)
        append (list key
                     (if (member key '(:panes :all-panes))
                         (mapcar #'layout-pane-metadata value)
                         (ekko/extensions::copy-data value)))))

(defun schedule-layout (worker state registry context token)
  "Queue one detached provider dispatch. TOKEN binds it to a live view epoch."
  (reset-layout-state state registry)
  (let* ((provider (layout-provider registry))
         (snapshot (and provider (layout-policy-context provider context))))
    (when (and provider worker (null (extension-worker-request worker))
               (or (not (equal snapshot (layout-state-requested-context state)))
                   (not (equal token (layout-state-requested-token state)))))
      (let ((request (make-layout-request :owner (getf provider :owner)
                                         :provider (getf provider :name)
                                         :reads (copy-list (getf provider :reads))
                                         :snapshot snapshot :token token)))
        ;; Mark a failed input as seen too. A bad callback must not starve
        ;; commands or keep restarting the worker for an unchanged context.
        (setf (layout-state-requested-context state) snapshot
              (layout-state-requested-token state) (copy-list token))
        (handler-case
            (extension-send (extension-worker-output worker)
                            (list :dispatch :layout (getf provider :name) snapshot
                                  (list :type :layout :version 1)))
          (error (condition)
            (setf (layout-state-error state) (princ-to-string condition))
            (error condition)))
        (setf (extension-worker-request worker) request
              (extension-worker-deadline worker) (+ (now) +layout-deadline+))
        request))))

(defun layout-point-p (value)
  (and (listp value) (= (length value) 2)
       (every (lambda (n) (and (integerp n)
                              (<= (- +layout-coordinate-limit+) n +layout-coordinate-limit+))) value)))

(defun validate-layout-actions (actions panes)
  "Return detached placements and camera only after the entire action is valid."
  (unless (and (listp actions) (= (length actions) 1)
               (consp (first actions)) (eq (caar actions) :place-panes))
    (error "A layout callback must return one :PLACE-PANES action"))
  (let* ((args (policy-plist (cdar actions) '(:version :placements :camera)
                             '(:version :placements :camera) "layout action"))
         (placements (getf args :placements)) (camera (getf args :camera))
         (ids (mapcar (lambda (pane) (getf pane :id)) panes)) (seen nil))
    (unless (and (eql (getf args :version) 1) (layout-point-p camera)
                 (listp placements) (<= (length placements) 128))
      (error "Invalid layout version, camera or placement count"))
    (dolist (placement placements)
      (policy-plist placement '(:pane :x :y :cols :rows :outer :visible)
                    '(:pane :x :y :cols :rows :outer) "pane placement")
      (let ((id (getf placement :pane)) (x (getf placement :x)) (y (getf placement :y))
            (cols (getf placement :cols)) (rows (getf placement :rows))
            (outer (getf placement :outer)))
        (unless (and (integerp id) (member id ids) (not (member id seen))
                     (member (getf placement :visible t) '(nil t))
                     (layout-point-p (list x y))
                     (typep cols '(integer 1 500)) (typep rows '(integer 1 300))
                     (listp outer) (= (length outer) 4)
                     (layout-point-p (subseq outer 0 2))
                     (typep (third outer) '(integer 1 532))
                     (typep (fourth outer) '(integer 1 332)))
          (error "Invalid pane identity, dimensions or logical rectangle"))
        (destructuring-bind (ox oy width height) outer
          (unless (and (<= ox x) (<= oy y)
                       (<= (+ x cols) (+ ox width)) (<= (+ y rows) (+ oy height)))
            (error "Pane content must fit its full logical outer rectangle")))
        (push id seen)))
    (values (ekko/extensions::copy-data placements) (copy-list camera))))

(defun layout-request-current-p (request registry context token)
  (let ((provider (layout-provider registry)))
    (and (equal token (layout-request-token request))
         (equal (getf provider :owner) (layout-request-owner request))
         (equal (getf provider :name) (layout-request-provider request))
         (equal (getf provider :reads) (layout-request-reads request))
         (equal (layout-policy-context provider context) (layout-request-snapshot request)))))

(defun complete-layout (state request registry context token response)
  "Validate a worker response and commit its owner contribution, never a VT."
  (unless (layout-request-current-p request registry context token)
    (return-from complete-layout :stale))
  (unless (and (listp response) (= (length response) 2) (eq (first response) :result))
    (error "Invalid layout callback response: ~S" response))
  (let ((result (policy-plist (second response) '(:owner :actions)
                              '(:owner :actions) "layout response")))
    (unless (equal (getf result :owner) (layout-request-owner request))
      (error "Layout callback returned the wrong owner"))
    (multiple-value-bind (placements camera)
        (validate-layout-actions (getf result :actions) (getf context :panes))
      (setf (layout-state-owner state) (layout-request-owner request)
            (layout-state-provider state) (layout-request-provider request)
            (layout-state-placements state) placements (layout-state-camera state) camera
            (layout-state-committed-context state) (layout-request-snapshot request)
            (layout-state-committed-token state) (copy-list token)
            (layout-state-error state) nil)))
  :committed)

(defun layout-state-current-p (state registry context token)
  "A successful logical layout must still match every declared dependency."
  (let ((provider (layout-provider registry)))
    (or (null provider)
        (and (equal (layout-state-owner state) (getf provider :owner))
             (equal (layout-state-provider state) (getf provider :name))
             (equal (layout-state-committed-token state) token)
             (equal (layout-state-committed-context state)
                    (layout-policy-context provider context))))))

(defun fail-layout (state request error)
  (when (and (equal (layout-state-owner state) (layout-request-owner request))
             (equal (layout-state-provider state) (layout-request-provider request)))
    (setf (layout-state-error state) (princ-to-string error)))
  nil)
