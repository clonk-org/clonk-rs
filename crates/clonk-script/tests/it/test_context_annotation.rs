// Test for context annotations with Condition= pattern

#[test]
fn context_annotation_with_condition() {
    let source = r#"
public func ContextTest(pObj)
{
  [$TxtTest$|Image=ELEC|Condition=IsNotInPermanentMode|Desc=$TxtDesc$]
  return 1;
}
    "#;
    crate::support::assert_compiles(source);
}

#[test]
fn elev_context_permanent_mode_turn_on() {
    // Exact from ELEV line 42-46
    let source = r#"
public func ContextPermanentModeTurnOn(pObj)
{
  [$TxtPermanentModeTurnOn$|Image=ELEC|Condition=IsNotInPermanentMode|Desc=$TxtPermanentModeDesc$]
  pCase->DoControlAuto(pObj);
}
    "#;
    crate::support::assert_compiles(source);
}

#[test]
fn full_elev_function_sequence() {
    // Full sequence from ELEV including both context functions and IsInPermanentMode
    let source = r#"
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
    "#;
    crate::support::assert_compiles(source);
}
