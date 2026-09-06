(in-package #:cl-user)

(defun customization-check (ok message)
  (unless ok (error "Customization: ~A" message)))

(defun customization-signals-error-p (thunk)
  (handler-case (progn (funcall thunk) nil)
    (error () t)))

(defun run-customization-tests ()
  ;; Mixed trees cover every cell once, apart from intentional column dividers.
  (let* ((tree (ekko/layout:split-pane (ekko/layout:split-pane 1 1 2 :columns) 2 3 :rows))
         (rects (ekko/layout:rectangles tree 81 30 1)))
    (customization-check (equal rects '((1 0 0 40 30) (2 41 0 40 15) (3 41 15 40 15))) "nested geometry")
    (customization-check (equal (ekko/layout:remove-pane tree 3) '(:columns 50 1 2)) "collapse closed branch")
    (customization-check (equal (ekko/layout:rectangles tree 2 3 3) '((3 0 0 2 3))) "small terminal focuses without losing tree")
    (customization-check (equal (ekko/layout:resize-split tree 3 10) '(:columns 50 1 (:rows 60 2 3))) "resize nearest ancestor")
    (customization-check (equal (ekko/layout:swap-panes tree 1 3) '(:columns 50 3 (:rows 50 2 1))) "swap stable IDs"))
  (let ((tree '(:columns 50 1 2)))
    (customization-check (equal (ekko/layout:minimum-size tree :leaf-min '(5 4) :column-gap 2 :row-gap 1)
                                '(12 4)) "custom minimum includes frame and gap")
    (customization-check (equal (ekko/layout:rectangles tree 20 6 1 nil :leaf-min '(5 4) :column-gap 2)
                                '((1 0 0 9 6) (2 11 0 9 6))) "custom column gap geometry")
    (customization-check (equal (ekko/layout:rectangles tree 4 4 2 nil :leaf-min '(5 4) :column-gap 2)
                                '((2 0 0 4 4))) "tiny viewport collapses before applying gap"))
  (let* ((pane (ekko/runtime::make-pane :id 1 :vt (ekko/vt:make-terminal :cols 10 :rows 4)))
         (session (ekko/runtime::make-session
                   :panes (list pane) :tree 1 :cols 10 :rows 8
                   :registry (list :options (list :pane-insets '(0 1 1 2)
                                                  :viewport-insets '(1 1 0 0)
                                                  :split-gaps '(0 0))))))
    (ekko/runtime::layout session)
    (let* ((scene (ekko/runtime::scene-data session))
           (record (first (seventh scene))))
      (customization-check
       (and (= 5 (first scene))
            (equal (subseq record 0 5) '(1 2 1 6 6))
            (equal (nth 12 record) '(0 1 9 7)))
       "server derives inset content and v5 outer rectangle")
      (ekko/runtime::install-registry
       session (list :api-version 1 :components nil :keymaps nil :commands nil :bindings nil
                     :options (list :pane-insets '(0 0 0 0) :viewport-insets '(0 0 1 0)
                                    :split-gaps '(0 0))))
      (customization-check
       (and (= 10 (ekko/vt:terminal-cols (ekko/runtime::pane-vt pane)))
            (= 7 (ekko/vt:terminal-rows (ekko/runtime::pane-vt pane)))
            (equal (list (ekko/runtime::pane-outer-x pane)
                         (ekko/runtime::pane-outer-y pane)
                         (ekko/runtime::pane-outer-cols pane)
                         (ekko/runtime::pane-outer-rows pane))
                   '(0 0 10 7)))
       "registry reload reapplies effective geometry immediately")))
  (let ((vt (ekko/vt:make-terminal :cols 8 :rows 3))
        (emit (lambda (&rest args) (declare (ignore args)))))
    (ekko/vt:feed vt (ekko/platform:text-bytes (format nil "one~C~%two~C~%three~C~%four" #\Return #\Return #\Return)) emit)
    (customization-check (equalp (ekko/vt:history-text vt) #("one" "two" "three" "four")) "scroll captures old row in order")
    (let ((frozen (ekko/vt:history-text vt)))
      (ekko/vt:feed vt (ekko/platform:text-bytes (format nil "~C[2Jchanged" #\Esc)) emit)
      (customization-check (equalp frozen #("one" "two" "three" "four")) "copy snapshot survives screen mutation"))
    (ekko/vt:feed vt (ekko/platform:text-bytes (format nil "~C[Hmain~C[?1049h~Calternate" #\Esc #\Esc #\Return)) emit)
    (ekko/vt:resize-terminal vt 6 4 8 16)
    (ekko/vt:feed vt (ekko/platform:text-bytes (format nil "~C[?1049l" #\Esc)) emit)
    (customization-check (string= (aref (ekko/vt:history-text vt) (ekko/vt:history-count (ekko/vt:terminal-history vt))) "main") "alternate resize preserves main screen")
    (let ((count (ekko/vt:history-count (ekko/vt:terminal-history vt))))
      (ekko/vt:feed vt (ekko/platform:text-bytes (format nil "~C[?1049h~20%~C[?1049l" #\Esc #\Esc)) emit)
      (customization-check (= count (ekko/vt:history-count (ekko/vt:terminal-history vt))) "alternate output excluded from history")))
  (let ((vt (ekko/vt:make-terminal :cols 1 :rows 1)))
    (dotimes (i 10005) (ekko/vt::remember-row vt (vector (list (write-to-string i) '(0)))))
    (customization-check (= 10000 (ekko/vt:history-count (ekko/vt:terminal-history vt))) "history row bound")
    (customization-check (string= "5" (aref (ekko/vt:history-text vt) 0)) "history evicts oldest"))
  (let ((vt (ekko/vt:make-terminal :cols 1 :rows 1))
        (row (make-array 500 :initial-element '("W" (0 38 2 255 255 255)))))
    (dotimes (i 1000) (ekko/vt::remember-row vt row))
    (customization-check (< (ekko/vt:history-count (ekko/vt:terminal-history vt)) 1000) "history byte bound evicts before row bound")
    (customization-check (<= (ekko/vt::history-bytes (ekko/vt:terminal-history vt)) (* 8 1024 1024)) "history accounted bytes"))
  (customization-check (string= "A界" (ekko/runtime::clip-copy-text "A界B" 3)) "copy clips by cells")
  (customization-check (string= "A" (ekko/runtime::clip-copy-text "A界B" 2)) "copy never cuts wide glyph")
  (let* ((panes (list (list 1 0 1 20 3 "upper" nil 0 0 nil nil nil '(0 0 20 4))
                      (list 2 0 5 20 3 "lower" nil 0 0 nil nil nil '(0 4 20 4))))
         (rows (ekko/runtime::scene-text-rows
                20 9 2 panes
                '(:status "custom" :style (0 32)
                  :decorations ((0 0 "upper" (0)) (0 4 "lower" (0))
                                (0 8 "custom" (0 32)))))))
    (customization-check (and (search "upper" (aref rows 0)) (not (search "lower" (aref rows 0)))
                              (search "lower" (aref rows 4)) (search "custom" (aref rows 8))) "horizontal split headers and status"))
  (let* ((panes (list (list 1 0 0 10 3 "topless" nil 0 0 nil nil nil '(0 0 10 3))
                      (list 2 10 0 10 3 "adjacent" nil 0 0 nil nil nil '(10 0 10 3))
                      (list 3 20 0 10 3 "farther" nil 0 0 nil nil nil '(20 0 10 3))))
         (rows (ekko/runtime::scene-text-rows 20 4 1 panes '(:status "hidden" :style (0 32)
                                                               :viewport-insets (0 0 0 0) :split-gaps (0 0)))))
    (customization-check (and (not (search "topless" (aref rows 0)))
                              (not (search "hidden" (aref rows 3)))
                              (not (search "│" (aref rows 0)))) "metadata suppresses absent chrome"))
  (let* ((panes (list (list 1 0 0 10 3 "left" nil 0 0 nil nil nil '(0 0 10 3))
                      (list 2 11 0 9 3 "right" nil 0 0 nil nil nil '(11 0 9 3))))
         (rows (ekko/runtime::scene-text-rows
                20 4 1 panes
                '(:status "" :style (0 32) :viewport-insets (0 0 0 0)
                  :decorations ((10 0 "│" (0 36)))))))
    (customization-check (search "│" (aref rows 0)) "metadata draws divider only in gap"))
  ;; Rebuilding owner contributions is the inverse: session state is deliberately
  ;; outside this registry, while every owned command/binding/option disappears.
  (let ((ekko/extensions::*components* nil))
    (ekko/builtins:install)
    (let ((before (ekko/extensions::registry)))
      (ekko/extensions:register-component :id :test :reads '(:focus))
      (ekko/extensions:register-keymap :component :test :name :test-map :unbound :ignore)
      (ekko/extensions:register-command :component :test :name "zoom" :handler (lambda (s e) (declare (ignore s e)) nil))
      (ekko/extensions:bind-key :component :test :map :test-map :key "z" :command "zoom")
      (ekko/extensions:set-option :component :test :name :initial-keymap :value :test-map)
      (ekko/extensions:set-option :component :test :name :initial-layout :value '(:columns 50 1 2))
      (ekko/extensions:set-option :component :test :name :prefix :value "C-a")
      (ekko/extensions:set-option :component :test :name :shell :value '("sh"))
      (ekko/extensions:set-option :component :test :name :status-text :value "test")
      (ekko/extensions:set-option :component :test :name :status-style :value '(0 31))
      (ekko/extensions:unregister-component :test)
      (customization-check (equal before (ekko/extensions::registry)) "unmount restores all registrations and shadowed defaults")))
  ;; Re-registering an owner starts with a clean contribution set. This catches
  ;; stale commands, maps, bindings and options surviving a config replacement.
  (let ((ekko/extensions::*components* nil))
    (ekko/extensions:register-component :id :test)
    (ekko/extensions:register-keymap :component :test :name :old-map)
    (ekko/extensions:register-command :component :test :name "old-command"
      :handler (lambda (s e) (declare (ignore s e)) nil))
    (ekko/extensions:bind-key :component :test :map :old-map :key "x" :command "old-command")
    (ekko/extensions:set-option :component :test :name :status-text :value "old")
    (ekko/extensions:register-component :id :test :reads '(:mode))
    (let* ((registry (ekko/extensions::registry))
           (component (first (getf registry :components))))
      (customization-check
       (and (equal (getf component :id) "test")
            (equal (getf registry :keymaps) nil)
            (equal (getf registry :commands) nil)
            (equal (getf registry :bindings) nil)
            (equal (getf (getf registry :options) :status-text) nil)
            (equal (getf component :reads) '(:mode)))
       "owner replacement reconstructs only the new contribution set")))
  ;; Registry validation is performed before installing anything. A malformed
  ;; map reference or initial map must leave the active session fully intact.
  (let* ((session (ekko/runtime::make-session :name "validation"))
         (valid (list :api-version 1
                      :components nil
                      :keymaps (list (list :name :normal :unbound :forward))
                      :commands nil :bindings nil
                      :options (list :initial-keymap :normal)))
         (bad-binding (list :api-version 1
                            :components nil
                            :keymaps (list (list :name :normal :unbound :forward))
                            :commands nil
                            :bindings (list (list :map :missing :key 120 :command nil))
                            :options (list :initial-keymap :normal)))
         (bad-initial (list :api-version 1
                            :components nil
                            :keymaps (list (list :name :normal :unbound :forward))
                            :commands nil :bindings nil
                            :options (list :initial-keymap :missing))))
    (ekko/runtime::install-registry session valid)
    (let ((old-registry (ekko/runtime::session-registry session))
          (old-mode (ekko/runtime::session-mode session))
          (old-generation (ekko/runtime::session-config-generation session))
          (old-revision (ekko/runtime::session-revision session)))
      (dolist (candidate (list bad-binding bad-initial))
        (customization-check (customization-signals-error-p
                              (lambda () (ekko/runtime::install-registry session candidate)))
                             "invalid registry rejected")
        (customization-check
         (and (eq old-registry (ekko/runtime::session-registry session))
              (eq old-mode (ekko/runtime::session-mode session))
              (= old-generation (ekko/runtime::session-config-generation session))
              (= old-revision (ekko/runtime::session-revision session)))
         "invalid registry preserves active session"))))
  ;; Public option and map registration reject bad values before changing the
  ;; owner's existing registry entries.
  (let ((ekko/extensions::*components* nil))
    (ekko/extensions:register-component :id :validation)
    (ekko/extensions:register-keymap :component :validation :name :valid-map)
    (ekko/extensions:set-option :component :validation :name :status-text :value "kept")
    (ekko/extensions:set-option :component :validation :name :pane-insets :value '(1 2 3 4))
    (ekko/extensions:set-option :component :validation :name :viewport-insets :value '(0 1 0 2))
    (ekko/extensions:set-option :component :validation :name :split-gaps :value '(2 1))
    (ekko/extensions:set-option :component :validation :name :erase-display-history :value t)
    (let ((before (ekko/extensions::registry)))
      (dolist (thunk (list
                       (lambda () (ekko/extensions:register-keymap :component :validation :name :prefix))
                       (lambda () (ekko/extensions:register-keymap :component :validation :name :valid-map :unbound :bad))
                       (lambda () (ekko/extensions:set-option :component :validation :name :initial-keymap :value :prefix))
                       (lambda () (ekko/extensions:set-option :component :validation :name :initial-keymap :value "valid-map"))
                       (lambda () (ekko/extensions:set-option :component :validation :name :pane-insets :value '(17 0 0 0)))
                       (lambda () (ekko/extensions:set-option :component :validation :name :split-gaps :value '(0)))
                       (lambda () (ekko/extensions:set-option :component :validation :name :initial-layout :value '(:rows 50 1 1)))
                       (lambda () (ekko/extensions:set-option :component :validation :name :erase-display-history :value :yes))))
        (customization-check (customization-signals-error-p thunk) "invalid map or initial option rejected"))
      (customization-check (equal before (ekko/extensions::registry)) "invalid registration preserves owner state")))
  (let* ((a (ekko/runtime::make-pane :id 1 :label "one" :vt (ekko/vt:make-terminal)))
         (b (ekko/runtime::make-pane :id 2 :label "two" :vt (ekko/vt:make-terminal)))
         (session (ekko/runtime::make-session :name "before" :panes (list a b) :tree '(:columns 50 1 2)
                    :registry (list :components
                                (loop for key in '(:session :focus :panes :layout :mode)
                                      collect (list :id key :reads (list key) :hook t))))))
    (ekko/runtime::schedule-hooks session)
    (customization-check (= 5 (length (ekko/runtime::session-hooks session))) "all initial dependency consumers")
    (dolist (key '(:session :focus :panes :layout :mode))
      (setf (ekko/runtime::session-hooks session) nil)
      (case key
        (:session (setf (ekko/runtime::session-name session) "after"))
        (:focus (setf (ekko/runtime::session-focus session) 1))
        (:panes (setf (ekko/runtime::pane-label a) "changed"))
        (:layout (setf (ekko/runtime::session-tree session) '(:rows 50 1 2)))
        (:mode (setf (ekko/runtime::session-mode session) :normal)))
      (ekko/runtime::schedule-hooks session)
      (customization-check
       (equal (sort (copy-list (ekko/runtime::session-hooks session)) #'string<)
              (sort (if (eq key :layout) (list :layout :panes) (list key)) #'string<))
       "exact declared consumers notified, including pane layout rectangles")))
  ;; A hook result is valid when unrelated context changed while it was
  ;; running, but a change to one of its declared reads must reject the stale
  ;; result and schedule that consumer again.
  (let* ((a (ekko/runtime::make-pane :id 1 :vt (ekko/vt:make-terminal)))
         (b (ekko/runtime::make-pane :id 2 :vt (ekko/vt:make-terminal)))
         (session (ekko/runtime::make-session
                   :panes (list a b) :tree '(:columns 50 1 2)
                   :registry (list :components
                                   (list (list :id "focus-hook" :reads '(:focus) :hook t)))))
         (snapshot (ekko/runtime::context-data session)))
    (setf (ekko/runtime::pane-label a) "output-changed")
    (customization-check
     (ekko/runtime::hook-context-current-p
      session (list :hook "focus-hook" snapshot))
     "unrelated pane output does not discard focus-only hook")
    (setf (ekko/runtime::session-focus session) 1)
    (customization-check
     (not (ekko/runtime::hook-context-current-p
           session (list :hook "focus-hook" snapshot)))
     "changed declared focus rejects stale hook")
    (setf (ekko/runtime::session-focus session) 0
          (ekko/runtime::session-hook-context session) (ekko/runtime::context-data session)
          (ekko/runtime::session-hooks session) nil
          (ekko/runtime::session-focus session) 1)
    (ekko/runtime::schedule-hooks session)
    (customization-check
     (equal (ekko/runtime::session-hooks session) (list "focus-hook"))
     "changed declared focus requeues hook"))
  (let* ((pane (ekko/runtime::make-pane :id 1 :vt (ekko/vt:make-terminal)))
         (session (ekko/runtime::make-session
                   :panes (list pane) :tree 1
                   :registry (list :components
                                   (list (list :id "constant-hook" :reads nil :hook t))))))
    (ekko/runtime::schedule-hooks session)
    (customization-check
     (equal (ekko/runtime::session-hooks session) (list "constant-hook"))
     "empty-read hook runs on initial context")
    (setf (ekko/runtime::session-hooks session) nil
          (ekko/runtime::pane-label pane) "unrelated")
    (ekko/runtime::schedule-hooks session)
    (customization-check
     (null (ekko/runtime::session-hooks session))
     "empty-read hook does not rerun for unrelated context"))
  ;; A primary action may complete with one trailing mode transition and status
  ;; contribution. Validation still happens for the whole batch first.
  (let* ((pane (ekko/runtime::make-pane :id 1 :label "one" :vt (ekko/vt:make-terminal)))
         (session (ekko/runtime::make-session :name "actions" :panes (list pane) :tree 1))
         (registry (list :api-version 1 :components nil
                         :keymaps (list (list :name :normal :unbound :forward)
                                        (list :name :locked :unbound :ignore))
                         :commands nil :bindings nil :options nil)))
    (ekko/runtime::install-registry session registry)
    (setf (ekko/runtime::session-mode session) :normal)
    (customization-check
     (not (customization-signals-error-p
           (lambda () (ekko/runtime::validate-actions
                       session (list (ekko/extensions:action :set-keymap :name :locked)) nil))))
     "set-keymap accepts a registered map")
    (ekko/runtime::apply-actions
     session :test
     (list (ekko/extensions:action :zoom)
           (ekko/extensions:action :set-keymap :name :locked)
           (ekko/extensions:action :status :text "zoomed")) nil nil)
    (customization-check (and (ekko/runtime::session-zoom session)
                              (eq :locked (ekko/runtime::session-mode session))
                              (equal '((:test . "zoomed"))
                                     (ekko/runtime::session-contributions session)))
                         "primary action completes before mode and status commits")
    (ekko/runtime::apply-actions
     session :test
     (list (ekko/extensions:action
            :decorate :spans (list (list :x 0 :y 0 :text "owned" :sgr '(0 31))))) nil nil)
    (customization-check
     (equal (cdr (assoc :test (ekko/runtime::session-decorations session)))
            '((:x 0 :y 0 :text "owned" :sgr (0 31))))
     "decorate replaces the owner's contribution")
    (ekko/runtime::apply-actions
     session :test
     (list (ekko/extensions:action
            :decorate :spans (list (list :x 0 :y 0 :text "chrome" :sgr '(0 31))
                                    (list :x 0 :y 1 :text "content" :sgr '(0 32))))) nil nil)
    (let* ((scene (ekko/runtime::scene-data session))
           (spans (getf (nth 7 scene) :decorations)))
      (customization-check
       (and (some (lambda (span) (and (= (second span) 0)
                                      (string= (third span) "chrome"))) spans)
            (not (some (lambda (span) (string= (third span) "content")) spans)))
       "scene decorations are clipped away from app content"))
    (setf (getf (ekko/runtime::session-registry session) :components)
          (list (list :id "early") (list :id "late"))
          (ekko/runtime::session-decorations session)
          (list (cons "late" '((:x 0 :y 0 :text "late" :sgr nil)))
                (cons "early" '((:x 0 :y 0 :text "early" :sgr nil)))))
    (let ((spans (getf (nth 7 (ekko/runtime::scene-data session)) :decorations)))
      (customization-check
       (equal (mapcar #'third spans) '("early" "late"))
       "decoration layers follow registry declaration order"))
    (ekko/runtime::apply-actions
     session "early"
     (list (ekko/extensions:action
            :decorate :spans (list (list :x 0 :y 0 :text "early-update" :sgr nil)))) nil nil)
    (let ((spans (getf (nth 7 (ekko/runtime::scene-data session)) :decorations)))
      (customization-check (equal (mapcar #'third spans) '("early-update" "late"))
                           "owner replacement preserves layer order"))
    (setf (ekko/runtime::session-decorations session)
          (remove "late" (ekko/runtime::session-decorations session)
                  :key #'car :test #'equal))
    (let ((spans (getf (nth 7 (ekko/runtime::scene-data session)) :decorations)))
      (customization-check (equal (mapcar #'third spans) '("early-update"))
                           "removing a later layer exposes the earlier layer"))
    (let ((before (copy-tree (ekko/runtime::session-decorations session))))
      (dolist (spans (list (list (list :x 500 :y 0 :text "bad" :sgr nil))
                           (list (list :x 0 :y 0 :text (format nil "ok~Cbad" #\Esc) :sgr nil))
                           (list (list :x 0 :y 0 :text "bad" :sgr '(0 256)))
                           (list (list :x 0 :y 0 :text "bad" :sgr nil :rows 0))
                           (list (list :x 0 :y 0 :text "bad" :sgr nil :rows 301))))
        (customization-check
         (customization-signals-error-p
          (lambda () (ekko/runtime::apply-actions
                      session :test
                      (list (ekko/extensions:action :decorate :spans spans)) nil nil)))
         "invalid decoration rejected"))
      (customization-check (equal before (ekko/runtime::session-decorations session))
                           "invalid decoration preserves owner state"))
    (setf (ekko/runtime::session-decorations session)
          (list (cons "full"
                      (loop repeat 1024 collect
                        '(:x 0 :y 0 :text "x" :sgr nil)))))
    (let ((before (copy-tree (ekko/runtime::session-decorations session))))
      (customization-check
       (customization-signals-error-p
        (lambda () (ekko/runtime::apply-actions
                    session :test
                    (list (ekko/extensions:action
                           :decorate :spans '((:x 0 :y 0 :text "new" :sgr nil)))) nil nil)))
       "retained decoration aggregate bound enforced")
      (customization-check (equal before (ekko/runtime::session-decorations session))
                           "aggregate rejection preserves existing owners"))
    (setf (ekko/runtime::session-mode session) :normal
          (ekko/runtime::session-zoom session) nil
          (ekko/runtime::session-contributions session) nil
          (ekko/runtime::session-decorations session) nil)
    (let ((before-revision (ekko/runtime::session-revision session)))
      (dolist (actions (list
                         (list (ekko/extensions:action :set-keymap :name :locked)
                               (ekko/extensions:action :zoom))
                         (list (ekko/extensions:action :zoom)
                               (ekko/extensions:action :set-keymap :name :locked)
                               (ekko/extensions:action :set-keymap :name :normal))
                         (list (ekko/extensions:action :zoom)
                               (ekko/extensions:action :focus-next))
                         (list (ekko/extensions:action :zoom)
                               (ekko/extensions:action :set-keymap :name :missing)
                               (ekko/extensions:action :status :text "should-not-appear"))))
        (customization-check
         (customization-signals-error-p
          (lambda () (ekko/runtime::apply-actions session :test actions nil nil)))
         "invalid transition order, count, or target rejected")
        (customization-check
         (and (eq :normal (ekko/runtime::session-mode session))
              (null (ekko/runtime::session-zoom session))
              (null (ekko/runtime::session-contributions session))
              (= before-revision (ekko/runtime::session-revision session)))
         "invalid compound batch leaves state unchanged")))
    ;; A fallible primary action must not commit its trailing mode or status.
    ;; Filling the pane queue reaches the real capacity check without mocking
    ;; any global function or process resource.
    (let* ((wire (ekko/runtime::make-wire :fd -1 :queued 65536))
           (failing-pane (ekko/runtime::make-pane :id 1 :label "one"
                                                   :io wire :vt (ekko/vt:make-terminal)))
           (failing (ekko/runtime::make-session :name "failing" :panes (list failing-pane) :tree 1
                                                :registry (ekko/runtime::session-registry session)
                                                :clipboard "x")))
      (setf (ekko/runtime::session-mode failing) :normal)
      (customization-check
       (customization-signals-error-p
        (lambda () (ekko/runtime::apply-actions
                    failing :test
                    (list (ekko/extensions:action :paste-buffer)
                          (ekko/extensions:action :set-keymap :name :locked)
                          (ekko/extensions:action :status :text "should-not-appear")) nil nil)))
       "primary action failure is reported")
      (customization-check
       (and (eq :normal (ekko/runtime::session-mode failing))
            (null (ekko/runtime::session-contributions failing))
            (= 65536 (ekko/runtime::wire-queued wire)))
       "primary failure leaves mode and status uncommitted"))
    (dolist (action (list (ekko/extensions:action :set-keymap :name :missing)
                          (ekko/extensions:action :set-keymap :name :normal :extra t)))
      (customization-check
       (customization-signals-error-p
        (lambda () (ekko/runtime::validate-actions session (list action) nil)))
       "set-keymap rejects unknown or extra arguments"))
    (customization-check
     (customization-signals-error-p
      (lambda () (ekko/runtime::apply-actions
                  session :test
                  (list (ekko/extensions:action :status :text "should-not-appear")
                        (ekko/extensions:action :set-keymap :name :missing)) nil nil)))
     "invalid set-keymap batch rejected")
    (customization-check (and (eq :normal (ekko/runtime::session-mode session))
                              (null (ekko/runtime::session-contributions session)))
                         "invalid set-keymap batch is atomic"))
  ;; Activation history belongs to the daemon, not a replaceable worker.
  (let* ((first (ekko/runtime::make-pane :id 1 :vt (ekko/vt:make-terminal)))
         (second (ekko/runtime::make-pane :id 2 :vt (ekko/vt:make-terminal)))
         (session (ekko/runtime::make-session :panes (list first second)
                                              :tree '(:columns 50 1 2)))
         (registry (list :api-version 1 :components nil :commands nil
                         :keymaps nil :bindings nil :options nil)))
    (ekko/runtime::record-activation session first)
    (ekko/runtime::set-focus session 1)
    (customization-check (> (ekko/runtime::pane-activation-order second)
                            (ekko/runtime::pane-activation-order first))
                         "focus records activation order")
    (let ((order (ekko/runtime::pane-activation-order second)))
      (ekko/runtime::set-focus session 1)
      (ekko/runtime::install-registry session registry)
      (customization-check (= order (ekko/runtime::pane-activation-order second))
                           "same focus and reload preserve activation history"))
    (ekko/runtime::set-focus session 0)
    (let ((snapshot (getf (ekko/runtime::context-data session) :panes)))
      (customization-check (> (getf (first snapshot) :activation-order)
                              (getf (second snapshot) :activation-order))
                           "public snapshot exposes daemon activation order")
      (setf (getf (first snapshot) :activation-order) -1)
      (customization-check (plusp (ekko/runtime::pane-activation-order first))
                           "activation snapshot is detached")))
  (dolist (tree '(nil 1 (:columns 50 1 (:rows 25 2 3)) (:rows 99 2 1)))
    (customization-check
     (not (customization-signals-error-p
           (lambda () (ekko/extensions::initial-layout-leaves tree))))
     "valid initial layout"))
  (dolist (tree '(0 17 2 (:columns 50 1 1) (:rows 0 1 2)
                  (:rows 100 1 2) (:diagonal 50 1 2) (:rows 50 1 3)
                  (:rows 50 1 2 3)))
    (customization-check
     (customization-signals-error-p
      (lambda () (ekko/extensions::initial-layout-leaves tree)))
     "invalid initial layout rejected"))
  (let* ((panes (loop for id from 1 to 3 collect
                  (ekko/runtime::make-pane :id id :vt (ekko/vt:make-terminal))))
         (tree '(:columns 50 1 (:rows 50 2 3)))
         (registry (list :api-version 1 :options
                         (list :pane-insets '(1 1 1 1) :viewport-insets '(0 0 0 0)
                               :split-gaps '(0 0) :initial-layout tree)))
         (session (ekko/runtime::make-session :panes panes :tree (copy-tree tree)
                                              :cols 80 :rows 24 :registry registry)))
    (ekko/runtime::layout session)
    (let ((before (mapcar (lambda (pane) (getf pane :layout-rect))
                         (getf (ekko/runtime::context-data session) :panes))))
      (ekko/runtime::apply-actions session :test '((:zoom)) nil nil)
      (ekko/runtime::apply-actions session :test '((:focus :pane 3)) nil nil)
      (let ((snapshot (getf (ekko/runtime::context-data session) :panes)))
        (customization-check
         (and (equal before (mapcar (lambda (pane) (getf pane :layout-rect)) snapshot))
              (not (getf (first snapshot) :visible))
              (= 38 (getf (first snapshot) :cols))
              (= 22 (getf (first snapshot) :rows))
              (getf (third snapshot) :visible)
              (equal '(0 0 80 24) (getf (third snapshot) :outer-rect)))
         "zoom focus retains public tiled geometry")))
    (dolist (target '(nil 3 99))
      (customization-check
       (customization-signals-error-p
        (lambda () (ekko/runtime::validate-actions session (list (list :close :focus target)) nil)))
       "close rejects absent or closing replacement"))
    (ekko/runtime::apply-actions session :test '((:close :focus 2)) nil nil)
    (customization-check
     (and (= 2 (ekko/runtime::pane-id (ekko/runtime::focused-pane session)))
          (equal '(:columns 50 1 2) (ekko/runtime::session-tree session)))
     "close selects requested survivor and collapses branch")
    (ekko/runtime::install-registry session registry)
    (customization-check
     (equal '(:columns 50 1 2) (ekko/runtime::session-tree session))
     "initial layout does not reset live tree on reload"))
  (let* ((pane (ekko/runtime::make-pane :id 1 :vt (ekko/vt:make-terminal)))
         (registry (list :api-version 1 :components
                         (list (list :id "notes" :reads '(:pane-notes) :hook t)
                               (list :id "other" :reads '(:focus) :hook t))))
         (session (ekko/runtime::make-session :panes (list pane) :tree 1 :registry registry))
         (action '(:pane-note :pane 1 :text "transient" :sgr (0 31) :duration 1000)))
    (ekko/runtime::schedule-hooks session)
    (setf (ekko/runtime::session-hooks session) nil)
    (ekko/runtime::apply-actions session "notes" (list action) nil nil)
    (ekko/runtime::schedule-hooks session)
    (customization-check (equal '("notes") (ekko/runtime::session-hooks session))
                         "pane note schedules declared consumers")
    (let* ((until (getf (first (ekko/runtime::session-pane-notes session)) :until))
           (snapshot (getf (ekko/runtime::context-data session) :pane-notes)))
      (customization-check (equal snapshot '((:owner "notes" :pane 1 :text "transient" :sgr (0 31))))
                           "pane note is public data")
      (setf (char (getf (first snapshot) :text) 0) #\X)
      (customization-check (string= "transient" (getf (first (ekko/runtime::session-pane-notes session)) :text))
                           "pane note snapshot is detached")
      (ekko/runtime::apply-actions session "notes" (list action) nil nil)
      (customization-check (= until (getf (first (ekko/runtime::session-pane-notes session)) :until))
                           "repeated active note does not extend duration")
      (dolist (invalid (list (list :pane-note :pane 99 :text "x" :sgr nil :duration 1000)
                            (list :pane-note :pane 1 :text (string #\Esc) :sgr nil :duration 1)
                            (list :pane-note :pane 1 :text "x" :sgr '(256) :duration 1)
                            (list :pane-note :pane 1 :text "x" :sgr nil :duration 0)))
        (customization-check
         (customization-signals-error-p
          (lambda () (ekko/runtime::apply-actions session "notes"
                       (list '(:status :text "uncommitted") invalid) nil nil)))
         "invalid note batch rejected atomically"))
      (customization-check (null (ekko/runtime::session-contributions session))
                           "invalid note leaves prior contributions untouched")
      (customization-check
       (customization-signals-error-p
        (lambda () (ekko/runtime::validate-actions session (list action) t)))
       "hooks cannot restart note timers")
      (ekko/runtime::expire-pane-notes session (- until 1/1000))
      (customization-check (ekko/runtime::session-pane-notes session) "note lives until deadline")
      (ekko/runtime::expire-pane-notes session until)
      (customization-check (null (ekko/runtime::session-pane-notes session)) "note expires at deadline"))
    (ekko/runtime::apply-actions session "notes" (list action) nil nil)
    (ekko/runtime::install-registry session registry)
    (customization-check (null (ekko/runtime::session-pane-notes session)) "reload removes transient owner notes")
    ;; Closing a pane prunes its note synchronously after the live tree is
    ;; committed, so no intermediate public snapshot can target a dead pane.
    (let* ((first-pane (ekko/runtime::make-pane :id 1 :vt (ekko/vt:make-terminal)))
           (second-pane (ekko/runtime::make-pane :id 2 :vt (ekko/vt:make-terminal)))
           (close-session
             (ekko/runtime::make-session
              :panes (list first-pane second-pane) :tree '(:columns 50 1 2)
              :focus 1 :cols 80 :rows 24 :registry registry)))
      (ekko/runtime::layout close-session)
      (ekko/runtime::apply-actions
       close-session "notes"
       '((:pane-note :pane 2 :text "closing" :sgr (0 31) :duration 1000)) nil nil)
      (customization-check (= 1 (length (ekko/runtime::session-pane-notes close-session)))
                           "close test installs pane note")
      (ekko/runtime::apply-actions close-session "notes" '((:close :focus 1)) nil nil)
      (customization-check (null (ekko/runtime::session-pane-notes close-session))
                           "close synchronously prunes removed pane note")))
  (format t "Customization, split layout and scrollback tests passed~%") t)
