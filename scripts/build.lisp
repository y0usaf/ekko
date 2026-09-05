(require "asdf")
(let* ((root (truename (or (uiop:getenv "EKKO_SOURCE_DIR") ".")))
       (asdf:*central-registry* nil))
  (push root asdf:*central-registry*)
  (asdf:load-system (or (uiop:getenv "EKKO_BUILD_SYSTEM") "ekko"))
  (sb-ext:save-lisp-and-die
   (or (uiop:getenv "EKKO_OUTPUT") "ekko")
   :toplevel (symbol-function (find-symbol "EXECUTABLE-MAIN" "EKKO"))
   :executable t
   :save-runtime-options t
   :compression 9))
