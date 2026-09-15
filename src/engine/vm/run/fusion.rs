//! Number-only execution of publication-authenticated canonical spans.
use super::{Error, Instruction, RunSlots};
use crate::engine::code::fusion::UpdateLocal;

pub(super) fn update_local(
    slots: &mut RunSlots<'_>,
    index: u16,
    update: UpdateLocal,
) -> Result<bool, Error> {
    if !slots.update_number_local(index, |previous| {
        let next = previous.update(update.increment);
        let result = (!update.discard).then_some(if update.postfix { previous } else { next });
        (next, result)
    })? {
        return Ok(false);
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(if update.discard {
        "fusion.UpdateLocalDiscard"
    } else if update.postfix {
        "fusion.UpdateLocalPostfix"
    } else {
        "fusion.UpdateLocalPrefix"
    });
    Ok(true)
}

pub(super) fn compare_branch(
    slots: &mut RunSlots<'_>,
    instruction: &Instruction,
    branch: &Instruction,
) -> Result<Option<usize>, Error> {
    if !matches!(
        instruction,
        Instruction::Lt
            | Instruction::Lte
            | Instruction::Gt
            | Instruction::Gte
            | Instruction::Eq
            | Instruction::StrictEq
            | Instruction::Neq
            | Instruction::StrictNeq
    ) {
        return Err(Error::internal("invalid authenticated comparison span"));
    }
    let (target, when) = match branch {
        Instruction::IfTrue(target) => (*target as usize, true),
        Instruction::IfFalse(target) => (*target as usize, false),
        _ => return Err(Error::internal("invalid authenticated comparison branch")),
    };
    let Some(result) = slots.consume_number_pair(|left, right| {
        let (left, right) = (left.float(), right.float());
        match instruction {
            Instruction::Lt => left < right,
            Instruction::Lte => left <= right,
            Instruction::Gt => left > right,
            Instruction::Gte => left >= right,
            Instruction::Eq | Instruction::StrictEq => left == right,
            Instruction::Neq | Instruction::StrictNeq => left != right,
            _ => unreachable!("comparison opcode was validated before the transaction"),
        }
    })?
    else {
        return Ok(None);
    };
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event("fusion.CompareBranch");
    // usize::MAX cannot be an instruction PC: executable allocation bounds
    // require the instruction array's byte length to fit isize::MAX.
    Ok(Some(if result == when { target } else { usize::MAX }))
}

/// Count the original logical operations, including intermediate stack depths.
/// This remains the authoritative span weight for future instruction budgets;
/// currently neither canonical run nor these spans has an instruction budget.
#[cfg(feature = "profiling")]
pub(super) fn record_span(code: &[Instruction], mut depth: usize) {
    for instruction in code {
        crate::engine::api::profiling::record_owned_instruction(depth);
        let effect = instruction.stack_contract();
        depth = depth - effect.popped + effect.pushed;
    }
}

#[cfg(all(test, feature = "profiling"))]
mod tests {
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn number_spans_execute_all_update_results_and_comparison_directions() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(context.eval("(()=>{ let n=1; let a=++n; let b=n++; n++; --n; for(let i=0;i<4;i++){n++;} return a===2 && b===2 && n===7 && !(NaN<0) && (-0===0); })()").unwrap(), Value::Bool(true));
        let costs = profile.snapshot();
        for event in [
            "fusion.UpdateLocalPrefix",
            "fusion.UpdateLocalPostfix",
            "fusion.UpdateLocalDiscard",
            "fusion.CompareBranch",
        ] {
            assert!(
                costs
                    .owned_execution_events
                    .get(event)
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "missing {event}: {costs:?}"
            );
        }
        assert_eq!(costs.owned_bridge_exits, 0);
    }

    #[test]
    fn guarded_fallback_keeps_coercion_const_tdz_and_capture_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let log=[];
            let n={valueOf(){log.push('convert'); return 4;}};
            let previous=n++;
            let big=2n; let old=big++;
            let text='7'; ++text;
            let captured=1; function read(){return captured;} captured++;
            let constant=false; try { const c={valueOf(){log.push('const');return 1;}}; c++; } catch(e){constant=e instanceof TypeError;}
            let tdz=false; try { z++; let z; } catch(e){tdz=e instanceof ReferenceError;}
            let compared={valueOf(){log.push('compare'); return 1;}};
            if(compared<2)log.push('branch');
            return previous===4 && n===5 && old===2n && big===3n && text===8 && read()===2 && constant && tdz && log.join(',')==='convert,const,compare,branch';
        })()"#).unwrap(), Value::Bool(true));
    }
    #[test]
    fn signed_zero_overflow_and_postfix_preserve_numeric_representation() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let negativeZero=-0; let previous=negativeZero++;
            let positiveZero=0; let positivePrevious=positiveZero--;
            let maximum=2147483647; let maximumPrevious=maximum++;
            let minimum=-2147483648; let minimumPrevious=minimum--;
            let infinity=Infinity; let infinityPrevious=infinity++;
            let nan=NaN; let nanPrevious=nan--;
            return Object.is(previous,-0) && negativeZero===1 && Object.is(positivePrevious,0)
                && positiveZero===-1 && maximumPrevious===2147483647 && maximum===2147483648
                && minimumPrevious===-2147483648 && minimum===-2147483649
                && infinityPrevious===Infinity && infinity===Infinity
                && Number.isNaN(nanPrevious) && Number.isNaN(nan)
                && !(nan<0) && !(nan>=0);
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn conversion_side_effects_and_resume_boundaries_keep_canonical_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let x={valueOf(){x=99; return 3;}}; let old=x++;
            let y={valueOf(){y=77; throw 42;}}; let caught=false;
            try {y++;} catch(error){caught=error===42;}
            let add=1; function change(){add=10;return 2;} add+=change();
            let read=1; let sum=read+(read=2);
            function* values(){let n=0;try {yield n++;}finally{++n;}return n++;}
            let iterator=values();let first=iterator.next();let second=iterator.next();
            let final=0;try{final++;throw 1;}catch(error){++final;}finally{final++;}
            return old===3 && x===4 && caught && y===77 && add===3 && sum===3 && read===2
                && first.value===0 && !first.done && second.value===2 && second.done && final===3;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }
}
