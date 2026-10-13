// Validation and feedback for the OCR editor, independent of the window.
export function correctionForm(view,text,busy=false){
  const normalized=String(text??'').trim().replace(/\s+/gu,' ');
  let error='';
  if(!/[\p{L}\p{N}]/u.test(normalized))error='Enter the text you can read in the snapshot.';
  else if([...normalized].length>4096||/[\u0000-\u001f\u007f]/u.test(normalized))error='Use at most 4,096 characters without control characters.';
  return {text:normalized,error,disabled:busy||!view||!!error,canRemember:!!view?.program};
}
