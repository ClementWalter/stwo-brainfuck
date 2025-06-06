use crate::components::{processor::table::ProcessorElements, JumpClaim};
use brainfuck_vm::instruction::InstructionType;
use num_traits::One;
use stwo_prover::{
    constraint_framework::{EvalAtRow, FrameworkComponent, FrameworkEval, RelationEntry},
    core::fields::m31::BaseField,
};

/// Implementation of `Component` and `ComponentProver` for the Jnz (Jump if not zero) `]`
/// instruction component.
///
/// It targets the `SimdBackend` from the Stwo constraint framework, with a
/// fallback on `CpuBakend` for small traces.
pub type JumpIfNotZeroComponent = FrameworkComponent<JumpIfNotZeroEval>;

/// The AIR for the [`JumpIfNotZeroComponent`].
///
/// Constraints are defined through the [`FrameworkEval`]
/// provided by the constraint framework of Stwo.
pub struct JumpIfNotZeroEval {
    /// The log size of the component's main trace height.
    log_size: u32,
    /// The random elements used for the lookup protocol linking the instruction logic components
    /// to the processor component.
    processor_lookup_elements: ProcessorElements,
}

impl JumpIfNotZeroEval {
    pub const fn new(claim: &JumpClaim, processor_lookup_elements: ProcessorElements) -> Self {
        Self { log_size: claim.log_size, processor_lookup_elements }
    }
}

impl FrameworkEval for JumpIfNotZeroEval {
    /// Returns the log size from the main claim.
    fn log_size(&self) -> u32 {
        self.log_size
    }

    /// The degree of the constraints is bounded by the size of the trace.
    ///
    /// Returns the ilog2 (upper) bound of the constraint degree for the component.
    fn max_constraint_log_degree_bound(&self) -> u32 {
        self.log_size + 1
    }

    /// Defines the AIR for the [`JumpIfNotZeroComponent`].
    ///
    /// Registers' values from the current row are obtained through masks.
    /// When you apply a mask, you target the current column and then pass to the next
    /// one: the register order matters to correctly fetch them.
    ///
    /// - Use `eval.next_trace_mask()` to get the current register from the main trace
    ///   (`ORIGINAL_TRACE_IDX`)
    ///
    /// Use `eval.add_constraint` to define a local constraint (boundary, transition).
    /// Use `eval.add_to_relation` to define a global constraint for the logUp protocol.
    ///
    /// The logUp must be finalized with `eval.finalize_logup()`.
    #[allow(clippy::similar_names)]
    fn evaluate<E: EvalAtRow>(&self, mut eval: E) -> E {
        let clk = eval.next_trace_mask();
        let ip = eval.next_trace_mask();
        let ci = eval.next_trace_mask();
        let ni = eval.next_trace_mask();
        let mp = eval.next_trace_mask();
        let mv = eval.next_trace_mask();
        let mvi = eval.next_trace_mask();
        let next_clk = eval.next_trace_mask();
        let next_ip = eval.next_trace_mask();
        let next_mp = eval.next_trace_mask();
        let next_mv = eval.next_trace_mask();
        let d = eval.next_trace_mask();
        let is_mv_zero = eval.next_trace_mask();

        // ┌─────────────────────────────┐
        // │   Consistency Constraints   │
        // └─────────────────────────────┘

        // `ci` is either 0 (dummy row), or equal to the JumpIfNotZero instruction `]`.
        let is_jump_if_not_zero_instruction =
            ci.clone() - InstructionType::JumpIfNotZero.to_base_field().into();

        eval.add_constraint(ci.clone() * is_jump_if_not_zero_instruction);

        // `clk` increments by one
        eval.add_constraint(next_clk - clk.clone() - E::F::one());

        // The dummy `d` is either 0 or 1
        eval.add_constraint(d.clone() * (d.clone() - E::F::one()));

        // If `d` is set, then `mv` equals 0
        eval.add_constraint(d.clone() * mv.clone());

        // If `d` is set, then `ci` equals 0
        eval.add_constraint(d.clone() * ci.clone());

        // ┌────────────────────────────┐
        // │   Transition Constraints   │
        // └────────────────────────────┘

        // Jump to `ni` if `mv != 0`. Otherwise, skip two.
        eval.add_constraint(
            (d.clone() - E::F::one()) *
                (is_mv_zero * (next_ip.clone() - ip.clone() - BaseField::from(2).into()) +
                    mv.clone() * (next_ip - ni.clone())),
        );

        // `mp` remains the same
        eval.add_constraint(next_mp - mp.clone());

        // `mv` remains the same
        eval.add_constraint(next_mv - mv.clone());

        // ┌─────────────────────────────┐
        // │   Interaction Constraints   │
        // └─────────────────────────────┘

        let num = d - E::F::one();

        eval.add_to_relation(RelationEntry::new(
            &self.processor_lookup_elements,
            num.into(),
            &[clk, ip, ci, ni, mp, mv, mvi],
        ));

        eval.finalize_logup();

        eval
    }
}

#[cfg(test)]
mod test {
    use crate::components::processor::{
        instructions::jump::{
            jump_if_not_zero_component::JumpIfNotZeroEval,
            table::{interaction_trace_evaluation, JumpIfNotZeroTable},
        },
        table::ProcessorElements,
    };
    use brainfuck_vm::{compiler::Compiler, test_helper::create_test_machine};
    use stwo_prover::{
        constraint_framework::{
            assert_constraints, preprocessed_columns::gen_is_first, FrameworkEval,
        },
        core::{
            pcs::TreeVec,
            poly::circle::{CanonicCoset, CircleEvaluation},
        },
    };

    #[test]
    fn test_jump_if_not_zero_constraints() {
        const LOG_SIZE: u32 = 6;

        // Get an execution trace from a valid Brainfuck program
        let code = "+++>,<[>+.<-]";
        let mut compiler = Compiler::new(code);
        let instructions = compiler.compile();
        let (mut machine, _) = create_test_machine(&instructions, &[1]);
        let () = machine.execute().expect("Failed to execute machine");

        let trace_vm = machine.trace();

        // Construct the IsFirst preprocessed column
        let is_first_col = gen_is_first(LOG_SIZE);
        let preprocessed_trace = vec![is_first_col];

        // Construct the main trace from the execution trace
        let table = JumpIfNotZeroTable::from(&trace_vm);
        let (main_trace, claim) = table.trace_evaluation().unwrap();

        // Draw Interaction elements
        let processor_lookup_elements = ProcessorElements::dummy();

        // Generate interaction trace
        let (interaction_trace, interaction_claim) =
            interaction_trace_evaluation(&main_trace, &processor_lookup_elements).unwrap();

        // Create the trace evaluation TreeVec
        let trace = TreeVec::new(vec![preprocessed_trace, main_trace, interaction_trace]);

        // Interpolate the trace for the evaluation
        let trace_polys = trace.map_cols(CircleEvaluation::interpolate);

        // Get the Memory AIR evaluator
        let jump_if_not_zero_eval = JumpIfNotZeroEval::new(&claim, processor_lookup_elements);

        // Assert that the constraints are valid for a valid Brainfuck program.
        assert_constraints(
            &trace_polys,
            CanonicCoset::new(LOG_SIZE),
            |eval| {
                jump_if_not_zero_eval.evaluate(eval);
            },
            (interaction_claim.claimed_sum, None),
        );
    }
}
