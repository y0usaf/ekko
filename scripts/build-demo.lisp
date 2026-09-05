(require "asdf")
(let ((root (truename (or (uiop:getenv "EKKO_SOURCE_DIR") "."))))
  (setf asdf:*central-registry* (list root))
  (asdf:load-system "ekko/graphics-demo")
  (sb-ext:save-lisp-and-die
   (or (uiop:getenv "EKKO_OUTPUT") "ekko-graphics-demo")
   :toplevel (symbol-function (find-symbol "EXECUTABLE-MAIN" "EKKO/GRAPHICS-DEMO"))
   :save-runtime-options t :executable t :compression 9))
