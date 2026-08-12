// Test for context annotations with Condition= pattern

crate::support::compile_case!(
    context_annotation_with_condition,
    r#"
public func ContextTest(pObj)
{
  [$TxtTest$|Image=ELEC|Condition=IsNotInPermanentMode|Desc=$TxtDesc$]
  return 1;
}
    "#,
);

// Exact from ELEV line 42-46
crate::support::compile_case!(
    elev_context_permanent_mode_turn_on,
    r#"
public func ContextPermanentModeTurnOn(pObj)
{
  [$TxtPermanentModeTurnOn$|Image=ELEC|Condition=IsNotInPermanentMode|Desc=$TxtPermanentModeDesc$]
  pCase->DoControlAuto(pObj);
}
    "#,
);

// Full sequence from ELEV including both context functions and IsInPermanentMode
crate::support::compile_case!(
    full_elev_function_sequence,
    r#"
local pCase;

public func ContextPermanentModeTurnOn(pObj)
{
  [$TxtPermanentModeTurnOn$|Image=ELEC|Condition=IsNotInPermanentMode|Desc=$TxtPermanentModeDesc$]
  pCase->DoControlAuto(pObj);
}

public func ContextPermanentModeTurnOff(pObj)
{
  [$TxtPermanentModeTurnOff$|Image=ELEC|Condition=IsInPermanentMode|Desc=$TxtPermanentModeDesc$]
  pCase->DoControlAuto(pObj);
}

public func IsInPermanentMode()
{
  if (!pCase) return(0);
  return (pCase->IsInPermanentMode());
}
    "#,
);
