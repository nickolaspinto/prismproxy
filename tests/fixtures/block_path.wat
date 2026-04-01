(module
  (memory (export "memory") 1)
  (func (export "on_request")
    (param $mp i32) (param $ml i32) (param $pp i32) (param $pl i32)
    (result i32)
    ;; Pass if path shorter than 6 bytes (length of "/block")
    (if (i32.lt_u (local.get $pl) (i32.const 6)) (then (return (i32.const 0))))
    ;; Check '/' = 47
    (if (i32.ne (i32.load8_u (local.get $pp))                          (i32.const 47))  (then (return (i32.const 0))))
    ;; Check 'b' = 98
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 1))) (i32.const 98))  (then (return (i32.const 0))))
    ;; Check 'l' = 108
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 2))) (i32.const 108)) (then (return (i32.const 0))))
    ;; Check 'o' = 111
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 3))) (i32.const 111)) (then (return (i32.const 0))))
    ;; Check 'c' = 99
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 4))) (i32.const 99))  (then (return (i32.const 0))))
    ;; Check 'k' = 107
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 5))) (i32.const 107)) (then (return (i32.const 0))))
    ;; All 6 bytes matched — block
    i32.const 1)
)
