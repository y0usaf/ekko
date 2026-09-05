;; Copy to ~/.config/ekko/init.lisp, or set EKKO_CONFIG to this file.
;; Then: ekko config check; ekko config reload SESSION.
(in-package #:cl-user)

(ekko/extensions:register-component
 :id :personal :reads '(:session :focus)
 :handler (lambda (snapshot event)
            (declare (ignore event))
            (list (ekko/extensions:action
                   :status :text
                   (format nil " ~A | pane ~D | C-a ? help"
                           (ekko/extensions:value snapshot :session)
                           (ekko/extensions:value snapshot :focus))))))

(ekko/extensions:set-option :component :personal :name :prefix :value "C-a")
(ekko/extensions:set-option :component :personal :name :status-style :value '(0 37 44))
(ekko/extensions:bind-key :component :personal :key "v" :command "split-columns")
(ekko/extensions:bind-key :component :personal :key "h" :command "split-rows")

(ekko/extensions:register-command
 :component :personal :name "label-work"
 :handler (lambda (snapshot event)
            (declare (ignore snapshot event))
            (list (ekko/extensions:action :rename :text "work"))))
(ekko/extensions:bind-key :component :personal :key "w" :command "label-work")
